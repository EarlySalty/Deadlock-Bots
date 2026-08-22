#![allow(clippy::result_large_err)]

mod faq_publish;
mod rang_guide_publish;
mod regelwerk_publish;
mod support_publish;
mod voice_ux_publish;
mod welcome_publish;

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, SecondsFormat, Utc};
use dl_discord::{BridgeAttachment, BridgeInteraction, BridgeReply, CommandSpec};
use dl_server_as_code::diff::{DiffAction, DiffChange, FieldDiff, ServerDiff};
use dl_server_as_code::{
    ApplyOptions, ApplyReport, BotMessageSpec, CategorySpec, ChannelSpec, GuildModel, ObjectKind,
    ObjectRef, PermissionOverwriteSpec, RoleSpec, SnapshotImportReport, TargetKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::{GuildId, Permissions};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

use crate::master;
pub use dl_voice::lfg_panel::LfgPanelApplyOutput;
pub use faq_publish::FaqPublishOutput;
pub use rang_guide_publish::RangGuidePublishOutput;
pub use support_publish::SupportPublishOutput;
pub use voice_ux_publish::VoiceUxPublishOutput as RouterApplyOutput;
pub use welcome_publish::{WelcomePublishOutput, WelcomeTeamMember};

pub const GUILD_ID: u64 = dl_server_as_code::DEFAULT_GUILD_ID;
pub const PORT: u16 = 8901;
pub const TOKEN_HEADER: &str = "X-Internal-Token";
const AUDIT_LOG_REASON: &str = "Onboarding-Redesign Welle 2a (Rechte-Sanierung)";
const ONBOARDING_AUDIT_LOG_REASON: &str = "serversync welle2b";
const ROLLBACK_VERSION: &str = "serversync.rollback_export.v2";
const ROLLBACK_VERSION_V1: &str = "serversync.rollback_export.v1";
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_DISCORD_CONTENT_CHARS: usize = 1800;
const MAX_BRIDGE_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
const ONBOARDING_DIFF_MESSAGE_KEY: &str = "native-onboarding";
const ONBOARDING_DIFF_OBJECT_ID: u64 = GUILD_ID;
const SERVERSYNC_KV_NS: &str = "serversync";
const WELLE2B_ARCHIVE_ENABLED_KEY: &str = "welle2b_archive_enabled";
const REGELWERK_MESSAGE_ID_KEY: &str = "regelwerk_message_id";
const RULES_CHANNEL_ID: u64 = dl_community::onboarding::RULES_CHANNEL_ID;
const SUPPORT_TICKET_CHANNEL_ID: u64 = 1_459_628_609_705_738_539;
const SERVER_GUIDE_DIFF_MESSAGE_KEY: &str = "server-guide";
const SERVER_GUIDE_DIFF_OBJECT_ID: u64 = GUILD_ID;
const SERVER_GUIDE_UNAVAILABLE_MESSAGE: &str =
    "Server Guide per API nicht verfügbar — manuelle Owner-Konfiguration nötig";
// docs.discord.food/resources/guild New Member Action Type: 0=VIEW, 1=CHAT.
const SERVER_GUIDE_ACTION_TYPE_VIEW: i64 = 0;
const SERVER_GUIDE_ACTION_TYPE_CHAT: i64 = 1;
const REGELWERK_DISCORD_MAX_ATTEMPTS: usize = 5;
const REGELWERK_DELETE_DELAY: Duration = Duration::from_millis(350);
const PREVIEW_MAX_AGE_MINUTES: i64 = 15;

// memes + deadlock-invite liegen seit W3.x im Archiv (kein @everyone-VIEW mehr);
// off-topic + gameplay-clips halten die Discord-Regel "mind. 5 Defaults mit
// @everyone VIEW+SEND" ein (live verifiziert 2026-07-15).
const DEFAULT_ONBOARDING_CHANNEL_NAMES: &[&str] = &[
    "allgemein",
    "frag-die-community",
    "mitspieler-suche",
    "off-topic",
    "gameplay-clips",
    "rank-ups",
    "patchnotes",
    "deadlock-rang",
    "server-support",
];

pub type SharedServerSync = Arc<dyn ServerSyncOps>;
type ServerSyncResult<T> = Result<T, ServerSyncError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerSyncErrorKind {
    BadRequest,
    Forbidden,
    Unauthorized,
    Internal,
}

#[derive(Debug, Clone)]
pub struct ServerSyncError {
    kind: ServerSyncErrorKind,
    message: String,
}

impl ServerSyncError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ServerSyncErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            kind: ServerSyncErrorKind::Forbidden,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            kind: ServerSyncErrorKind::Unauthorized,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ServerSyncErrorKind::Internal,
            message: message.into(),
        }
    }

    fn status(&self) -> StatusCode {
        match self.kind {
            ServerSyncErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ServerSyncErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ServerSyncErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
            ServerSyncErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl std::fmt::Display for ServerSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServerSyncError {}

impl From<dl_server_as_code::ServerAsCodeError> for ServerSyncError {
    fn from(value: dl_server_as_code::ServerAsCodeError) -> Self {
        match value {
            dl_server_as_code::ServerAsCodeError::PreviewExpired => {
                Self::bad_request(value.to_string())
            }
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<sqlx::Error> for ServerSyncError {
    fn from(value: sqlx::Error) -> Self {
        Self::internal(value.to_string())
    }
}

impl From<serde_json::Error> for ServerSyncError {
    fn from(value: serde_json::Error) -> Self {
        Self::internal(value.to_string())
    }
}

impl From<reqwest::Error> for ServerSyncError {
    fn from(value: reqwest::Error) -> Self {
        Self::internal(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotOutput {
    pub snapshot_id: i64,
    pub guild_id: u64,
    pub captured_at: String,
    pub categories: usize,
    pub channels: usize,
    pub roles: usize,
    pub overwrites: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemberRoleAssignment {
    pub member_id: u64,
    pub role_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollbackGuildModel {
    pub guild_id: u64,
    pub categories: Vec<CategorySpec>,
    pub channels: Vec<ChannelSpec>,
    pub roles: Vec<RoleSpec>,
    pub overwrites: Vec<PermissionOverwriteSpec>,
    pub bot_messages: Vec<BotMessageSpec>,
}

impl RollbackGuildModel {
    fn from_model(model: &GuildModel) -> Self {
        Self {
            guild_id: model.guild_id,
            categories: model.categories.values().cloned().collect(),
            channels: model.channels.values().cloned().collect(),
            roles: model.roles.values().cloned().collect(),
            overwrites: model.overwrites.values().cloned().collect(),
            bot_messages: model.bot_messages.values().cloned().collect(),
        }
    }

    fn to_model(&self) -> ServerSyncResult<GuildModel> {
        let mut model = GuildModel::new(self.guild_id);

        model.categories = collect_unique_by(&self.categories, "Kategorie", |category| {
            ensure_spec_guild(
                "Kategorie",
                category.category_id,
                category.guild_id,
                self.guild_id,
            )?;
            Ok(category.category_id)
        })?;
        model.channels = collect_unique_by(&self.channels, "Kanal", |channel| {
            ensure_spec_guild("Kanal", channel.channel_id, channel.guild_id, self.guild_id)?;
            Ok(channel.channel_id)
        })?;
        model.roles = collect_unique_by(&self.roles, "Rolle", |role| {
            ensure_spec_guild("Rolle", role.role_id, role.guild_id, self.guild_id)?;
            Ok(role.role_id)
        })?;
        model.overwrites = collect_unique_by(&self.overwrites, "Rechte-Overwrite", |overwrite| {
            ensure_spec_guild(
                "Rechte-Overwrite",
                overwrite.key.channel_id,
                overwrite.guild_id,
                self.guild_id,
            )?;
            Ok(overwrite.key.clone())
        })?;
        model.bot_messages = collect_unique_by(&self.bot_messages, "Bot-Nachricht", |message| {
            ensure_spec_guild(
                "Bot-Nachricht",
                message.channel_id,
                message.guild_id,
                self.guild_id,
            )?;
            Ok((message.channel_id, message.message_key.clone()))
        })?;

        Ok(model)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackArtifact {
    pub version: String,
    pub guild_id: u64,
    pub created_at: String,
    pub snapshot_id: i64,
    pub structure_snapshot: RollbackGuildModel,
    pub dynamic_namespaces: Vec<dl_server_as_code::DynamicNamespace>,
    pub documented_exceptions: Vec<dl_server_as_code::DocumentedException>,
    pub member_role_assignments: Vec<MemberRoleAssignment>,
    pub native_onboarding_config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackExportOutput {
    pub rollback_export_id: i64,
    pub snapshot_id: i64,
    pub guild_id: u64,
    pub artifact_hash: String,
    pub captured_at: String,
    pub members: usize,
    pub member_role_edges: usize,
    pub filename: String,
    pub artifact_text: String,
    pub artifact: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOutput {
    pub preview_id: i64,
    pub guild_id: u64,
    pub diff_hash: String,
    pub human_summary: String,
    pub warnings: Vec<String>,
    pub diff_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveFlagOutput {
    pub guild_id: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkPublishOutput {
    pub guild_id: u64,
    pub dry_run: bool,
    pub threads_found: usize,
    pub threads_deleted: usize,
    pub bot_messages_found: usize,
    pub bot_messages_deleted: usize,
    pub bot_message_ids: Vec<u64>,
    pub bot_message_embed_titles: Vec<String>,
    pub stored_message_id: Option<u64>,
    pub posted_message_id: Option<u64>,
    pub edited_message_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegelwerkBotMessage {
    message_id: u64,
    embed_titles: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RangGuideDeleteOutcome {
    deleted: Vec<u64>,
    warnings: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct StaticV2DeleteOutcome {
    deleted: Vec<u64>,
    warnings: Vec<String>,
}

struct RegelwerkOutputInput<'a> {
    dry_run: bool,
    thread_count: usize,
    threads_deleted: usize,
    bot_messages: &'a [RegelwerkBotMessage],
    bot_messages_deleted: usize,
    stored_message_id: Option<u64>,
    posted_message_id: Option<u64>,
    edited_message_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreOutput {
    pub rollback_export_id: i64,
    pub preview_id: i64,
    pub guild_id: u64,
    pub artifact_hash: String,
    pub diff_hash: String,
    pub human_summary: String,
    pub warnings: Vec<String>,
    pub diff_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyOutput {
    pub apply_run_id: i64,
    pub preview_id: i64,
    pub dry_run: bool,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub details_text: String,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeOnboardingConfig {
    pub prompts: Vec<NativeOnboardingPrompt>,
    pub default_channel_ids: Vec<String>,
    pub enabled: bool,
    pub mode: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeOnboardingPrompt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub prompt_type: u8,
    pub title: String,
    pub options: Vec<NativeOnboardingOption>,
    pub single_select: bool,
    pub required: bool,
    pub in_onboarding: bool,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeOnboardingOption {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<Value>,
    #[serde(default)]
    pub role_ids: Vec<String>,
    #[serde(default)]
    pub channel_ids: Vec<String>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct DiscordCurrentUser {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DiscordThreadPage {
    #[serde(default)]
    threads: Vec<DiscordThread>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct DiscordThread {
    id: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    thread_metadata: Option<DiscordThreadMetadata>,
}

#[derive(Debug, Deserialize)]
struct DiscordThreadMetadata {
    archive_timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordMessage {
    id: String,
    author: DiscordMessageAuthor,
    #[serde(default)]
    content: String,
    #[serde(default)]
    flags: u64,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    embeds: Vec<DiscordMessageEmbed>,
    #[serde(default)]
    components: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct DiscordMessageAuthor {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DiscordMessageEmbed {
    title: Option<String>,
    #[serde(default)]
    footer: Option<DiscordMessageEmbedFooter>,
}

#[derive(Debug, Deserialize)]
struct DiscordMessageEmbedFooter {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordMessageWriteResponse {
    id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct NativeOnboardingPutConfig {
    prompts: Vec<NativeOnboardingPutPrompt>,
    default_channel_ids: Vec<String>,
    enabled: bool,
    mode: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct NativeOnboardingPutPrompt {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type")]
    prompt_type: u8,
    title: String,
    options: Vec<NativeOnboardingPutOption>,
    single_select: bool,
    required: bool,
    in_onboarding: bool,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct NativeOnboardingPutOption {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji_animated: Option<bool>,
    #[serde(default)]
    role_ids: Vec<String>,
    #[serde(default)]
    channel_ids: Vec<String>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingBuildOutput {
    pub config: NativeOnboardingConfig,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnboardingPreviewOutput {
    pub preview_id: Option<i64>,
    pub guild_id: u64,
    pub diff_hash: Option<String>,
    pub human_summary: String,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub diff_text: String,
    pub desired_config: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGuideConfig {
    pub enabled: bool,
    pub welcome_message: ServerGuideWelcomeMessage,
    pub new_member_actions: Vec<ServerGuideAction>,
    pub resource_channels: Vec<ServerGuideResourceChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGuideWelcomeMessage {
    pub author_ids: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGuideAction {
    pub channel_id: String,
    pub action_type: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGuideResourceChannel {
    pub channel_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerGuideBuildOutput {
    pub config: Option<ServerGuideConfig>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGuidePreviewOutput {
    pub preview_id: Option<i64>,
    pub guild_id: u64,
    pub diff_hash: Option<String>,
    pub human_summary: String,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub diff_text: String,
    pub desired_config: Option<Value>,
}

#[async_trait]
pub trait ServerSyncOps: Send + Sync {
    fn guild_id(&self) -> u64;
    async fn snapshot(&self, requested_by_user_id: Option<u64>)
        -> ServerSyncResult<SnapshotOutput>;
    async fn rollback_export(
        &self,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<RollbackExportOutput>;
    async fn diff(&self, requested_by_user_id: Option<u64>) -> ServerSyncResult<DiffOutput>;
    async fn restore(
        &self,
        rollback_export_id: Option<i64>,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<RestoreOutput>;
    async fn apply(
        &self,
        preview_id: i64,
        hash: String,
        confirm: bool,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ApplyOutput>;
    async fn onboarding_preview(
        &self,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<OnboardingPreviewOutput>;
    async fn onboarding_apply(
        &self,
        preview_id: i64,
        hash: String,
        confirm: bool,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ApplyOutput>;
    async fn archive_enable(&self) -> ServerSyncResult<ArchiveFlagOutput>;
    async fn archive_disable(&self) -> ServerSyncResult<ArchiveFlagOutput>;
    async fn regelwerk_publish(&self, confirm: bool) -> ServerSyncResult<RegelwerkPublishOutput>;
    async fn regelwerk_apply(
        &self,
        confirm: bool,
    ) -> ServerSyncResult<regelwerk_publish::RegelwerkPublishOutput>;
    async fn support_apply(&self, confirm: bool) -> ServerSyncResult<SupportPublishOutput>;
    async fn faq_apply(&self, confirm: bool) -> ServerSyncResult<FaqPublishOutput>;
    async fn welcome_preview(&self) -> ServerSyncResult<WelcomePublishOutput>;
    async fn welcome_apply(&self, confirm: bool) -> ServerSyncResult<WelcomePublishOutput>;
    async fn rang_guide_apply(&self, confirm: bool) -> ServerSyncResult<RangGuidePublishOutput>;
    async fn load_voice_ux_message_ids(&self) -> ServerSyncResult<BTreeMap<u64, Vec<u64>>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn load_voice_ux_payload_formats(
        &self,
    ) -> ServerSyncResult<BTreeMap<u64, Option<String>>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn load_voice_ux_payload_hashes(
        &self,
    ) -> ServerSyncResult<BTreeMap<u64, Option<String>>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn load_voice_ux_lfg_forum_thread_id(&self) -> ServerSyncResult<Option<u64>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn fetch_voice_ux_v2_message_ids(
        &self,
        _channel_id: u64,
        _bot_user_id: u64,
    ) -> ServerSyncResult<Vec<u64>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn edit_voice_ux_message(
        &self,
        _channel_id: u64,
        _message_id: u64,
        _message: &voice_ux_publish::VoiceUxMessageOutput,
    ) -> ServerSyncResult<Option<u64>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn post_voice_ux_message(
        &self,
        _channel_id: u64,
        _message: &voice_ux_publish::VoiceUxMessageOutput,
    ) -> ServerSyncResult<u64> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn replace_voice_ux_target_metadata(
        &self,
        _target: &voice_ux_publish::VoiceUxTargetOutput,
        _message_ids: &[u64],
    ) -> ServerSyncResult<()> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn store_voice_ux_target_metadata(
        &self,
        _target: &voice_ux_publish::VoiceUxTargetOutput,
    ) -> ServerSyncResult<()> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn delete_voice_ux_messages(
        &self,
        _channel_id: u64,
        _stored_message_ids: &[u64],
        _bot_user_id: u64,
    ) -> ServerSyncResult<StaticV2DeleteOutcome> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn cleanup_new_voice_ux_posts_best_effort(&self, _channel_id: u64, _message_ids: &[u64]) {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn pin_voice_ux_message_best_effort(
        &self,
        _channel_id: u64,
        _message_id: u64,
    ) -> Result<(), String> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn fetch_voice_ux_legacy_cleanup_candidates(
        &self,
        _bot_user_id: u64,
    ) -> ServerSyncResult<(
        Vec<voice_ux_publish::VoiceUxLegacyCleanupCandidate>,
        Vec<String>,
    )> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn cleanup_voice_ux_legacy_messages(
        &self,
        _output: &mut voice_ux_publish::VoiceUxPublishOutput,
    ) -> ServerSyncResult<()> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn post_voice_ux_forum_post(
        &self,
        _forum: &voice_ux_publish::VoiceUxForumPostOutput,
    ) -> ServerSyncResult<u64> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn edit_voice_ux_forum_post(
        &self,
        _thread_id: u64,
        _forum: &voice_ux_publish::VoiceUxForumPostOutput,
    ) -> ServerSyncResult<Option<u64>> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn store_voice_ux_forum_metadata(
        &self,
        _thread_id: u64,
        _payload_hash: &str,
    ) -> ServerSyncResult<()> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn pin_voice_ux_forum_thread_best_effort(&self, _thread_id: u64) -> Result<(), String> {
        unreachable!("only ServerSyncService uses Voice-UX internals")
    }
    async fn router_apply(&self, confirm: bool) -> ServerSyncResult<RouterApplyOutput>;
    async fn lfg_panel_apply(&self, confirm: bool) -> ServerSyncResult<LfgPanelApplyOutput>;
    async fn serverguide_preview(
        &self,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ServerGuidePreviewOutput>;
    async fn serverguide_apply(
        &self,
        preview_id: i64,
        hash: String,
        confirm: bool,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ApplyOutput>;
}

pub struct ServerSyncService {
    pool: PgPool,
    adapter: Arc<dl_discord::DiscordAdapter>,
    discord_token: String,
    guild_id: u64,
    http_client: reqwest::Client,
    discord_api_base: String,
    rang_guide_repo_root: PathBuf,
    router_interface: tokio::sync::RwLock<Option<Arc<dl_voice::router::RouterInterface>>>,
    lfg_panel_interface: tokio::sync::RwLock<Option<Arc<dl_voice::lfg_panel::LfgPanelInterface>>>,
}

impl ServerSyncService {
    pub fn new(
        pool: PgPool,
        adapter: Arc<dl_discord::DiscordAdapter>,
        discord_token: String,
        guild_id: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            adapter,
            discord_token,
            guild_id,
            http_client: reqwest::Client::new(),
            discord_api_base: DISCORD_API_BASE.to_string(),
            rang_guide_repo_root: rang_guide_publish::rang_guide_repo_root(),
            router_interface: tokio::sync::RwLock::new(None),
            lfg_panel_interface: tokio::sync::RwLock::new(None),
        })
    }

    #[cfg(test)]
    fn new_for_test(
        pool: PgPool,
        discord_api_base: String,
        rang_guide_repo_root: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            adapter: dl_discord::DiscordAdapter::new("test-token"),
            discord_token: "test-token".to_string(),
            guild_id: GUILD_ID,
            http_client: reqwest::Client::new(),
            discord_api_base,
            rang_guide_repo_root,
            router_interface: tokio::sync::RwLock::new(None),
            lfg_panel_interface: tokio::sync::RwLock::new(None),
        })
    }

    pub async fn set_router_interface(&self, interface: Arc<dl_voice::router::RouterInterface>) {
        *self.router_interface.write().await = Some(interface);
    }

    pub async fn set_lfg_panel_interface(
        &self,
        interface: Arc<dl_voice::lfg_panel::LfgPanelInterface>,
    ) {
        *self.lfg_panel_interface.write().await = Some(interface);
    }

    fn discord_api_url(&self, path: &str) -> String {
        format!("{}{}", self.discord_api_base.trim_end_matches('/'), path)
    }

    async fn fetch_member_role_assignments(&self) -> ServerSyncResult<Vec<MemberRoleAssignment>> {
        let guild_id = GuildId::new(self.guild_id);
        let mut after = None;
        let mut assignments = Vec::new();
        loop {
            let page = self
                .adapter
                .http
                .get_guild_members(guild_id, Some(1000), after)
                .await
                .map_err(|err| ServerSyncError::internal(err.to_string()))?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|member| member.user.id.get());
            assignments.extend(page.into_iter().map(|member| {
                let mut role_ids: Vec<u64> = member.roles.iter().map(|role| role.get()).collect();
                role_ids.sort_unstable();
                MemberRoleAssignment {
                    member_id: member.user.id.get(),
                    role_ids,
                }
            }));
            if assignments.last().is_none() || after.is_none() {
                break;
            }
            if assignments.len() % 1000 != 0 {
                break;
            }
        }
        assignments.sort_unstable_by_key(|entry| entry.member_id);
        Ok(assignments)
    }

    async fn fetch_native_onboarding_config(&self) -> ServerSyncResult<Value> {
        let url = format!("{DISCORD_API_BASE}/guilds/{}/onboarding", self.guild_id);
        let response = self
            .http_client
            .get(url)
            .header("Authorization", format!("Bot {}", self.discord_token))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body_preview: String = body.chars().take(300).collect();
            return Err(ServerSyncError::internal(format!(
                "Discord onboarding GET fehlgeschlagen: HTTP {}: {}",
                status.as_u16(),
                body_preview
            )));
        }
        Ok(response.json::<Value>().await?)
    }

    async fn put_native_onboarding_config(
        &self,
        config: &NativeOnboardingConfig,
    ) -> ServerSyncResult<()> {
        let url = format!("{DISCORD_API_BASE}/guilds/{}/onboarding", self.guild_id);
        let payload = native_onboarding_put_payload(config);
        let response = self
            .http_client
            .put(url)
            .header("Authorization", format!("Bot {}", self.discord_token))
            .header("Content-Type", "application/json")
            .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
            .json(&payload)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body_preview: String = body.chars().take(300).collect();
            return Err(ServerSyncError::internal(format!(
                "Discord onboarding PUT fehlgeschlagen: HTTP {}: {}",
                status.as_u16(),
                body_preview
            )));
        }
        Ok(())
    }

    async fn fetch_server_guide_config(&self) -> ServerSyncResult<Value> {
        let url = format!(
            "{DISCORD_API_BASE}/guilds/{}/new-member-welcome",
            self.guild_id
        );
        let response = self
            .http_client
            .get(url)
            .header("Authorization", format!("Bot {}", self.discord_token))
            .send()
            .await?;
        server_guide_response_json(response, "GET").await
    }

    async fn put_server_guide_config(&self, config: &ServerGuideConfig) -> ServerSyncResult<()> {
        let url = format!(
            "{DISCORD_API_BASE}/guilds/{}/new-member-welcome",
            self.guild_id
        );
        let response = self
            .http_client
            .put(url)
            .header("Authorization", format!("Bot {}", self.discord_token))
            .header("Content-Type", "application/json")
            .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
            .json(config)
            .send()
            .await?;
        let _: Value = server_guide_response_json(response, "PUT").await?;
        Ok(())
    }

    async fn welle2b_archive_enabled(&self) -> ServerSyncResult<bool> {
        let value = self.load_serversync_kv(WELLE2B_ARCHIVE_ENABLED_KEY).await?;
        Ok(value.as_deref() == Some("1"))
    }

    async fn set_welle2b_archive_enabled(
        &self,
        enabled: bool,
    ) -> ServerSyncResult<ArchiveFlagOutput> {
        if enabled {
            self.store_serversync_kv(WELLE2B_ARCHIVE_ENABLED_KEY, "1")
                .await?;
        } else {
            self.delete_serversync_kv(WELLE2B_ARCHIVE_ENABLED_KEY)
                .await?;
        }
        Ok(ArchiveFlagOutput {
            guild_id: self.guild_id,
            enabled,
        })
    }

    async fn load_serversync_kv(&self, key: &str) -> ServerSyncResult<Option<String>> {
        let value = sqlx::query_scalar(
            "SELECT v
               FROM bot.kv_store
              WHERE ns = $1
                AND k = $2",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(value)
    }

    async fn store_serversync_kv(&self, key: &str, value: &str) -> ServerSyncResult<()> {
        sqlx::query(
            "INSERT INTO bot.kv_store(ns, k, v)
             VALUES($1, $2, $3)
             ON CONFLICT(ns, k) DO UPDATE SET v = EXCLUDED.v",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_serversync_kv(&self, key: &str) -> ServerSyncResult<()> {
        sqlx::query(
            "DELETE FROM bot.kv_store
              WHERE ns = $1
                AND k = $2",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_serversync_kv_prefix(
        &self,
        prefix: &str,
    ) -> ServerSyncResult<BTreeMap<String, String>> {
        let rows = sqlx::query(
            "SELECT k, v
               FROM bot.kv_store
              WHERE ns = $1
                AND k LIKE $2
              ORDER BY k",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(format!("{prefix}%"))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let key: String = row.get("k");
                let value: String = row.get("v");
                (key, value)
            })
            .collect())
    }

    async fn load_static_v2_message_ids(&self, prefix: &str) -> ServerSyncResult<Vec<u64>> {
        let indexed = self.load_serversync_kv_prefix(prefix).await?;
        let mut by_index = BTreeMap::<usize, u64>::new();
        for (key, raw) in indexed {
            let raw_index = key.trim_start_matches(prefix);
            let Ok(message_index) = raw_index.parse::<usize>() else {
                continue;
            };
            by_index.insert(message_index, parse_discord_id(&key, &raw)?);
        }
        Ok(by_index.into_values().collect())
    }

    async fn store_static_v2_message_id(
        &self,
        prefix: &str,
        message_index: usize,
        message_id: u64,
    ) -> ServerSyncResult<()> {
        self.store_serversync_kv(&format!("{prefix}{message_index}"), &message_id.to_string())
            .await
    }

    async fn store_static_v2_metadata(
        &self,
        format_key: &str,
        payload_format: &str,
        hash_key: &str,
        payload_hash: &str,
    ) -> ServerSyncResult<()> {
        self.store_serversync_kv(format_key, payload_format).await?;
        self.store_serversync_kv(hash_key, payload_hash).await
    }

    async fn replace_static_v2_message_ids_and_metadata(
        &self,
        message_id_prefix: &str,
        format_key: &str,
        payload_format: &str,
        hash_key: &str,
        payload_hash: &str,
        message_ids: &[u64],
    ) -> ServerSyncResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM bot.kv_store
              WHERE ns = $1
                AND (k LIKE $2 OR k = $3 OR k = $4)",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(format!("{message_id_prefix}%"))
        .bind(format_key)
        .bind(hash_key)
        .execute(&mut *tx)
        .await?;

        for (message_index, message_id) in message_ids.iter().copied().enumerate() {
            sqlx::query(
                "INSERT INTO bot.kv_store(ns, k, v)
                 VALUES($1, $2, $3)",
            )
            .bind(SERVERSYNC_KV_NS)
            .bind(format!("{message_id_prefix}{message_index}"))
            .bind(message_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO bot.kv_store(ns, k, v)
             VALUES($1, $2, $3), ($1, $4, $5)",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(format_key)
        .bind(payload_format)
        .bind(hash_key)
        .bind(payload_hash)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn discord_get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: String,
    ) -> ServerSyncResult<T> {
        let response = self.discord_get_response(url).await?;
        discord_regelwerk_json_response(response, "GET")
    }

    async fn discord_get_response(&self, url: String) -> ServerSyncResult<DiscordRestResponse> {
        discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .get(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token));
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "GET",
        )
        .await
    }

    async fn discord_post_json<T: serde::de::DeserializeOwned>(
        &self,
        url: String,
        payload: Value,
    ) -> ServerSyncResult<T> {
        let response = discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .post(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("Content-Type", "application/json")
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                    .json(&payload);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "POST",
        )
        .await?;
        discord_regelwerk_json_response(response, "POST")
    }

    async fn discord_delete(&self, url: String) -> ServerSyncResult<bool> {
        let response = discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .delete(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "DELETE",
        )
        .await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !status.is_success() {
            return Err(ServerSyncError::internal(format!(
                "Discord DELETE fehlgeschlagen: HTTP {}: {}",
                status.as_u16(),
                response.body_preview()
            )));
        }
        Ok(true)
    }

    async fn fetch_current_bot_user_id(&self) -> ServerSyncResult<u64> {
        let user: DiscordCurrentUser = self
            .discord_get_json(self.discord_api_url("/users/@me"))
            .await?;
        parse_discord_id("Bot-User-ID", &user.id)
    }

    async fn fetch_regelwerk_threads(&self) -> ServerSyncResult<Vec<u64>> {
        let mut ids = Vec::new();
        // API v10 kennt aktive Threads nur noch guild-weit (der Kanal-Endpoint
        // liefert 404); daher guild-weit holen und auf den Regelwerk-Kanal filtern.
        let active: DiscordThreadPage = self
            .discord_get_json(format!(
                "{DISCORD_API_BASE}/guilds/{}/threads/active",
                self.guild_id
            ))
            .await?;
        let rules_channel_id = RULES_CHANNEL_ID.to_string();
        let active_in_rules = active
            .threads
            .into_iter()
            .filter(|thread| thread.parent_id.as_deref() == Some(rules_channel_id.as_str()))
            .collect::<Vec<_>>();
        ids.extend(thread_ids(active_in_rules)?);

        for endpoint in ["archived/public", "archived/private"] {
            let mut before: Option<String> = None;
            loop {
                let mut url = format!(
                    "{DISCORD_API_BASE}/channels/{RULES_CHANNEL_ID}/threads/{endpoint}?limit=100"
                );
                if let Some(before) = before.as_deref() {
                    url.push_str("&before=");
                    // archive_timestamp enthaelt `+00:00` — das `+` muss
                    // percent-encodiert werden, sonst liest Discord ein Leerzeichen.
                    url.push_str(&before.replace('+', "%2B"));
                }
                let page: DiscordThreadPage = self.discord_get_json(url).await?;
                let next_before = page
                    .threads
                    .last()
                    .and_then(|thread| thread.thread_metadata.as_ref())
                    .and_then(|metadata| metadata.archive_timestamp.clone());
                ids.extend(thread_ids(page.threads)?);
                if !page.has_more || next_before.is_none() {
                    break;
                }
                before = next_before;
            }
        }

        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    async fn fetch_regelwerk_bot_messages(
        &self,
        bot_user_id: u64,
        stored_message_id: Option<u64>,
    ) -> ServerSyncResult<Vec<RegelwerkBotMessage>> {
        let mut messages = Vec::new();
        let mut before: Option<u64> = None;
        loop {
            let mut url =
                format!("{DISCORD_API_BASE}/channels/{RULES_CHANNEL_ID}/messages?limit=100");
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                if author_id == bot_user_id && Some(message_id) != stored_message_id {
                    messages.push(RegelwerkBotMessage {
                        message_id,
                        embed_titles: message
                            .embeds
                            .iter()
                            .filter_map(|embed| embed.title.clone())
                            .collect(),
                    });
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        Ok(messages)
    }

    async fn load_regelwerk_message_id(&self) -> ServerSyncResult<Option<u64>> {
        self.load_serversync_kv(REGELWERK_MESSAGE_ID_KEY)
            .await?
            .map(|value| parse_discord_id("regelwerk_message_id", &value))
            .transpose()
    }

    async fn store_regelwerk_message_id(&self, message_id: u64) -> ServerSyncResult<()> {
        self.store_serversync_kv(REGELWERK_MESSAGE_ID_KEY, &message_id.to_string())
            .await
    }

    async fn edit_regelwerk_message(
        &self,
        message_id: u64,
        content: &str,
    ) -> ServerSyncResult<Option<u64>> {
        let url = format!("{DISCORD_API_BASE}/channels/{RULES_CHANNEL_ID}/messages/{message_id}");
        let payload = json!({ "content": content });
        let response = discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .patch(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("Content-Type", "application/json")
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                    .json(&payload);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "PATCH",
        )
        .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let message: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &message.id).map(Some)
    }

    async fn post_regelwerk_message(&self, content: &str) -> ServerSyncResult<u64> {
        let message: DiscordMessageWriteResponse = self
            .discord_post_json(
                format!("{DISCORD_API_BASE}/channels/{RULES_CHANNEL_ID}/messages"),
                json!({ "content": content }),
            )
            .await?;
        parse_discord_id("Message-ID", &message.id)
    }

    async fn fetch_welcome_team_members(&self) -> ServerSyncResult<Vec<WelcomeTeamMember>> {
        let guild_id = GuildId::new(self.guild_id);
        let mut after = None;
        let mut members = Vec::new();
        loop {
            let page = self
                .adapter
                .http
                .get_guild_members(guild_id, Some(1000), after)
                .await
                .map_err(|err| ServerSyncError::internal(err.to_string()))?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|member| member.user.id.get());
            members.extend(page.into_iter().map(|member| {
                let mut role_ids: Vec<u64> = member.roles.iter().map(|role| role.get()).collect();
                role_ids.sort_unstable();
                WelcomeTeamMember {
                    user_id: member.user.id.get(),
                    role_ids,
                    bot: member.user.bot,
                }
            }));
            if members.last().is_none() || after.is_none() {
                break;
            }
            if members.len() % 1000 != 0 {
                break;
            }
        }
        members.sort_unstable_by_key(|entry| entry.user_id);
        Ok(members)
    }

    async fn load_welcome_payload_format(&self) -> ServerSyncResult<Option<String>> {
        self.load_serversync_kv(welcome_publish::WELCOME_PAYLOAD_FORMAT_KEY)
            .await
    }

    async fn store_welcome_payload_format(&self) -> ServerSyncResult<()> {
        self.store_serversync_kv(
            welcome_publish::WELCOME_PAYLOAD_FORMAT_KEY,
            welcome_publish::WELCOME_PAYLOAD_FORMAT,
        )
        .await
    }

    async fn load_welcome_message_ids(&self) -> ServerSyncResult<BTreeMap<String, Vec<u64>>> {
        let indexed = self
            .load_serversync_kv_prefix("welcome_message_id_")
            .await?;
        let mut ids = BTreeMap::new();
        for section in welcome_publish::welcome_sections() {
            let mut by_index = BTreeMap::<usize, u64>::new();
            let legacy_key = welcome_publish::welcome_legacy_message_id_key(section.id);
            if let Some(raw) = indexed.get(&legacy_key) {
                by_index.insert(0, parse_discord_id(&legacy_key, raw)?);
            }

            let prefix = welcome_publish::welcome_message_id_key_prefix(section.id);
            for (key, raw) in indexed
                .iter()
                .filter(|(key, _)| key.starts_with(prefix.as_str()))
            {
                let raw_index = key.trim_start_matches(prefix.as_str());
                let Ok(message_index) = raw_index.parse::<usize>() else {
                    continue;
                };
                by_index.insert(message_index, parse_discord_id(key, raw)?);
            }
            if !by_index.is_empty() {
                ids.insert(section.id.to_string(), by_index.into_values().collect());
            }
        }
        Ok(ids)
    }

    async fn store_welcome_message_id(
        &self,
        section_id: &str,
        message_index: usize,
        message_id: u64,
    ) -> ServerSyncResult<()> {
        self.store_serversync_kv(
            &welcome_publish::welcome_message_id_key(section_id, message_index),
            &message_id.to_string(),
        )
        .await?;
        if message_index == 0 {
            self.delete_serversync_kv(&welcome_publish::welcome_legacy_message_id_key(section_id))
                .await?;
        }
        Ok(())
    }

    async fn clear_welcome_message_id_keys(
        &self,
        _stored_message_ids: &BTreeMap<String, Vec<u64>>,
    ) -> ServerSyncResult<()> {
        let keys = self
            .load_serversync_kv_prefix("welcome_message_id_")
            .await?
            .into_keys()
            .collect::<Vec<_>>();
        for key in keys {
            self.delete_serversync_kv(&key).await?;
        }
        Ok(())
    }

    async fn load_rang_guide_payload_format(&self) -> ServerSyncResult<Option<String>> {
        self.load_serversync_kv(rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY)
            .await
    }

    async fn load_rang_guide_payload_hash(&self) -> ServerSyncResult<Option<String>> {
        self.load_serversync_kv(rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY)
            .await
    }

    async fn load_rang_guide_message_ids(&self) -> ServerSyncResult<Vec<u64>> {
        let indexed = self
            .load_serversync_kv_prefix(rang_guide_publish::RANG_GUIDE_MESSAGE_ID_PREFIX)
            .await?;
        let mut by_index = BTreeMap::<usize, u64>::new();
        for (key, raw) in indexed {
            let raw_index =
                key.trim_start_matches(rang_guide_publish::RANG_GUIDE_MESSAGE_ID_PREFIX);
            let Ok(message_index) = raw_index.parse::<usize>() else {
                continue;
            };
            by_index.insert(message_index, parse_discord_id(&key, &raw)?);
        }
        Ok(by_index.into_values().collect())
    }

    async fn store_rang_guide_message_id(
        &self,
        message_index: usize,
        message_id: u64,
    ) -> ServerSyncResult<()> {
        self.store_serversync_kv(
            &rang_guide_publish::rang_guide_message_id_key(message_index),
            &message_id.to_string(),
        )
        .await
    }

    async fn store_rang_guide_metadata(
        &self,
        output: &rang_guide_publish::RangGuidePublishOutput,
    ) -> ServerSyncResult<()> {
        self.store_serversync_kv(
            rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY,
            rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT,
        )
        .await?;
        self.store_serversync_kv(
            rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY,
            &output.payload_hash,
        )
        .await
    }

    async fn replace_rang_guide_message_ids_and_metadata(
        &self,
        output: &rang_guide_publish::RangGuidePublishOutput,
        message_ids: &[u64],
    ) -> ServerSyncResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM bot.kv_store
              WHERE ns = $1
                AND (k LIKE $2 OR k = $3 OR k = $4)",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(format!(
            "{}%",
            rang_guide_publish::RANG_GUIDE_MESSAGE_ID_PREFIX
        ))
        .bind(rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY)
        .bind(rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY)
        .execute(&mut *tx)
        .await?;

        for (message_index, message_id) in message_ids.iter().copied().enumerate() {
            sqlx::query(
                "INSERT INTO bot.kv_store(ns, k, v)
                 VALUES($1, $2, $3)",
            )
            .bind(SERVERSYNC_KV_NS)
            .bind(rang_guide_publish::rang_guide_message_id_key(message_index))
            .bind(message_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO bot.kv_store(ns, k, v)
             VALUES($1, $2, $3), ($1, $4, $5)",
        )
        .bind(SERVERSYNC_KV_NS)
        .bind(rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY)
        .bind(rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT)
        .bind(rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY)
        .bind(&output.payload_hash)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn delete_rang_guide_messages(
        &self,
        stored_message_ids: &[u64],
        bot_user_id: u64,
    ) -> ServerSyncResult<RangGuideDeleteOutcome> {
        let mut message_ids = stored_message_ids.to_vec();
        message_ids.sort_unstable();
        message_ids.dedup();

        let mut deleted = Vec::new();
        let mut warnings = Vec::new();
        for message_id in message_ids {
            let fetch_url = self.discord_api_url(&format!(
                "/channels/{}/messages/{message_id}",
                rang_guide_publish::RANG_GUIDE_CHANNEL_ID
            ));
            let response = match self.discord_get_response(fetch_url).await {
                Ok(response) => response,
                Err(err) => {
                    warnings.push(format!(
                        "Rang-Guide: gespeicherte Message-ID {message_id} konnte vor Delete nicht gelesen werden: {err}"
                    ));
                    continue;
                }
            };
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                warnings.push(format!(
                    "Rang-Guide: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
                continue;
            }
            let message: DiscordMessage = match discord_regelwerk_json_response(response, "GET") {
                Ok(message) => message,
                Err(err) => {
                    warnings.push(format!(
                        "Rang-Guide: gespeicherte Message-ID {message_id} konnte vor Delete nicht validiert werden: {err}"
                    ));
                    continue;
                }
            };
            let author_id = match parse_discord_id("Message-Author-ID", &message.author.id) {
                Ok(author_id) => author_id,
                Err(err) => {
                    warnings.push(format!(
                        "Rang-Guide: gespeicherte Message-ID {message_id} hat ungueltigen Autor und wird nicht geloescht: {err}"
                    ));
                    continue;
                }
            };
            let candidate = rang_guide_publish::RangGuideV2Message {
                message_id,
                author_id,
                flags: message.flags,
                components: message.components,
            };
            if !rang_guide_publish::is_rang_guide_v2_message(&candidate, bot_user_id) {
                warnings.push(format!(
                    "Rang-Guide: gespeicherte Message-ID {message_id} passt nicht zur eigenen V2-Signatur und wird nicht geloescht"
                ));
                continue;
            }

            if self
                .discord_delete(self.discord_api_url(&format!(
                    "/channels/{}/messages/{message_id}",
                    rang_guide_publish::RANG_GUIDE_CHANNEL_ID
                )))
                .await?
            {
                deleted.push(message_id);
            } else {
                warnings.push(format!(
                    "Rang-Guide: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
            }
        }
        Ok(RangGuideDeleteOutcome { deleted, warnings })
    }

    async fn cleanup_new_rang_guide_posts_best_effort(&self, message_ids: &[u64]) {
        for &message_id in message_ids {
            match self
                .discord_delete(self.discord_api_url(&format!(
                    "/channels/{}/messages/{message_id}",
                    rang_guide_publish::RANG_GUIDE_CHANNEL_ID
                )))
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(
                        %err,
                        message_id,
                        "Rang-Guide: neu gepostete Message konnte nach Teilfehler nicht bereinigt werden"
                    );
                }
            }
        }
    }

    fn adopt_rang_guide_message_ids(
        output: &mut rang_guide_publish::RangGuidePublishOutput,
        discovered_message_ids: &[u64],
        confirm: bool,
    ) {
        output.stored_message_ids = discovered_message_ids.to_vec();
        output.repost_required = false;
        output.warnings.push(format!(
            "Rang-Guide: KV-Message-IDs fehlen oder sind unvollstaendig; vorhandene eigene V2-Messages werden adoptiert: {:?}",
            discovered_message_ids
        ));
        for (message_index, message_id) in discovered_message_ids.iter().copied().enumerate() {
            if let Some(message) = output.messages.get_mut(message_index) {
                message.stored_message_id = Some(message_id);
                message.message_id = Some(message_id);
                if !confirm {
                    message.action = "planned_adopted_edit".to_string();
                }
            }
        }
    }

    async fn edit_rang_guide_message(
        &self,
        message_id: u64,
        message: &rang_guide_publish::RangGuideMessageOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages/{message_id}",
            rang_guide_publish::RANG_GUIDE_CHANNEL_ID
        ));
        let response = self
            .send_rang_guide_message_payload("PATCH", url, &message.payload)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &written.id).map(Some)
    }

    async fn post_rang_guide_message(
        &self,
        message: &rang_guide_publish::RangGuideMessageOutput,
    ) -> ServerSyncResult<u64> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages",
            rang_guide_publish::RANG_GUIDE_CHANNEL_ID
        ));
        let response = self
            .send_rang_guide_message_payload("POST", url, &message.payload)
            .await?;
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Message-ID", &written.id)
    }

    async fn send_rang_guide_message_payload(
        &self,
        method: &'static str,
        url: String,
        payload: &rang_guide_publish::RangGuideMessagePayload,
    ) -> ServerSyncResult<DiscordRestResponse> {
        let payload_value = serde_json::to_value(payload)?;
        if !payload.attachments.is_empty() {
            let payload_text = serde_json::to_string(&payload_value)?;
            let mut files = Vec::new();
            for attachment in &payload.attachments {
                let path = self.rang_guide_repo_root.join(&attachment.relative_path);
                let bytes = std::fs::read(&path).map_err(|err| {
                    ServerSyncError::internal(format!(
                        "Rang-Guide-Attachment `{}` konnte nicht gelesen werden: {err}",
                        path.display()
                    ))
                })?;
                files.push((attachment.id, attachment.filename.clone(), bytes));
            }
            return discord_regelwerk_send_with_retry(
                || {
                    let request = match method {
                        "POST" => self.http_client.post(url.clone()),
                        "PATCH" => self.http_client.patch(url.clone()),
                        other => unreachable!("unsupported Discord method {other}"),
                    }
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON);
                    let payload_text = payload_text.clone();
                    let files = files.clone();
                    async move {
                        let mut form =
                            reqwest::multipart::Form::new().text("payload_json", payload_text);
                        for (id, filename, bytes) in files {
                            let part = reqwest::multipart::Part::bytes(bytes)
                                .file_name(filename)
                                .mime_str("image/png")?;
                            form = form.part(format!("files[{id}]"), part);
                        }
                        let response = request.multipart(form).send().await?;
                        DiscordRestResponse::from_response(response).await
                    }
                },
                method,
            )
            .await;
        }

        discord_regelwerk_send_with_retry(
            || {
                let request = match method {
                    "POST" => self.http_client.post(url.clone()),
                    "PATCH" => self.http_client.patch(url.clone()),
                    other => unreachable!("unsupported Discord method {other}"),
                }
                .header("Authorization", format!("Bot {}", self.discord_token))
                .header("Content-Type", "application/json")
                .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                .json(&payload_value);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            method,
        )
        .await
    }

    async fn send_components_v2_message_payload(
        &self,
        label: &'static str,
        method: &'static str,
        url: String,
        payload_value: Value,
        attachments: Vec<(u8, String, String)>,
    ) -> ServerSyncResult<DiscordRestResponse> {
        if !attachments.is_empty() {
            let payload_text = serde_json::to_string(&payload_value)?;
            let mut files = Vec::new();
            for (id, filename, relative_path) in attachments {
                let path = self.rang_guide_repo_root.join(&relative_path);
                let bytes = std::fs::read(&path).map_err(|err| {
                    ServerSyncError::internal(format!(
                        "{label}-Attachment `{}` konnte nicht gelesen werden: {err}",
                        path.display()
                    ))
                })?;
                files.push((id, filename, bytes));
            }
            return discord_regelwerk_send_with_retry(
                || {
                    let request = match method {
                        "POST" => self.http_client.post(url.clone()),
                        "PATCH" => self.http_client.patch(url.clone()),
                        other => unreachable!("unsupported Discord method {other}"),
                    }
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON);
                    let payload_text = payload_text.clone();
                    let files = files.clone();
                    async move {
                        let mut form =
                            reqwest::multipart::Form::new().text("payload_json", payload_text);
                        for (id, filename, bytes) in files {
                            let part = reqwest::multipart::Part::bytes(bytes)
                                .file_name(filename)
                                .mime_str("image/png")?;
                            form = form.part(format!("files[{id}]"), part);
                        }
                        let response = request.multipart(form).send().await?;
                        DiscordRestResponse::from_response(response).await
                    }
                },
                method,
            )
            .await;
        }

        discord_regelwerk_send_with_retry(
            || {
                let request = match method {
                    "POST" => self.http_client.post(url.clone()),
                    "PATCH" => self.http_client.patch(url.clone()),
                    other => unreachable!("unsupported Discord method {other}"),
                }
                .header("Authorization", format!("Bot {}", self.discord_token))
                .header("Content-Type", "application/json")
                .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                .json(&payload_value);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            method,
        )
        .await
    }

    async fn edit_regelwerk_v2_message(
        &self,
        message_id: u64,
        message: &regelwerk_publish::RegelwerkMessageOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages/{message_id}",
            regelwerk_publish::REGELWERK_CHANNEL_ID
        ));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload(
                "Regelwerk",
                "PATCH",
                url,
                payload_value,
                attachments,
            )
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &written.id).map(Some)
    }

    async fn post_regelwerk_v2_message(
        &self,
        message: &regelwerk_publish::RegelwerkMessageOutput,
    ) -> ServerSyncResult<u64> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages",
            regelwerk_publish::REGELWERK_CHANNEL_ID
        ));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload(
                "Regelwerk",
                "POST",
                url,
                payload_value,
                attachments,
            )
            .await?;
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Message-ID", &written.id)
    }

    async fn edit_support_message(
        &self,
        message_id: u64,
        message: &support_publish::SupportMessageOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages/{message_id}",
            support_publish::SUPPORT_CHANNEL_ID
        ));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload("Support", "PATCH", url, payload_value, attachments)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &written.id).map(Some)
    }

    async fn post_support_message(
        &self,
        message: &support_publish::SupportMessageOutput,
    ) -> ServerSyncResult<u64> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages",
            support_publish::SUPPORT_CHANNEL_ID
        ));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload("Support", "POST", url, payload_value, attachments)
            .await?;
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Message-ID", &written.id)
    }

    async fn edit_faq_message(
        &self,
        message_id: u64,
        message: &faq_publish::FaqMessageOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages/{message_id}",
            faq_publish::FAQ_CHANNEL_ID
        ));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload("FAQ", "PATCH", url, payload_value, attachments)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &written.id).map(Some)
    }

    async fn post_faq_message(
        &self,
        message: &faq_publish::FaqMessageOutput,
    ) -> ServerSyncResult<u64> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages",
            faq_publish::FAQ_CHANNEL_ID
        ));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload("FAQ", "POST", url, payload_value, attachments)
            .await?;
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Message-ID", &written.id)
    }

    async fn fetch_rang_guide_legacy_cleanup_candidates(
        &self,
        bot_user_id: u64,
    ) -> ServerSyncResult<Vec<rang_guide_publish::RangGuideLegacyCleanupCandidate>> {
        let mut before: Option<u64> = None;
        let mut candidates = Vec::new();
        for _ in 0..5 {
            let mut url = format!(
                "{}/channels/{}/messages?limit=100",
                self.discord_api_base.trim_end_matches('/'),
                rang_guide_publish::RANG_GUIDE_CHANNEL_ID
            );
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                let custom_ids =
                    rang_guide_publish::collect_component_custom_ids(&message.components);
                let legacy = rang_guide_publish::RangGuideLegacyMessage {
                    message_id,
                    author_id,
                    flags: message.flags,
                    has_embeds: !message.embeds.is_empty(),
                    custom_ids,
                };
                if rang_guide_publish::is_legacy_rank_guide_cleanup_candidate(&legacy, bot_user_id)
                {
                    candidates.push(rang_guide_publish::RangGuideLegacyCleanupCandidate {
                        message_id,
                        custom_ids: legacy.custom_ids,
                    });
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        candidates.sort_unstable_by_key(|candidate| candidate.message_id);
        candidates.dedup_by_key(|candidate| candidate.message_id);
        Ok(candidates)
    }

    async fn fetch_rang_guide_v2_message_ids(
        &self,
        bot_user_id: u64,
    ) -> ServerSyncResult<Vec<u64>> {
        let mut before: Option<u64> = None;
        let mut message_ids = Vec::new();
        for _ in 0..5 {
            let mut url = format!(
                "{}/channels/{}/messages?limit=100",
                self.discord_api_base.trim_end_matches('/'),
                rang_guide_publish::RANG_GUIDE_CHANNEL_ID
            );
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                let candidate = rang_guide_publish::RangGuideV2Message {
                    message_id,
                    author_id,
                    flags: message.flags,
                    components: message.components.clone(),
                };
                if rang_guide_publish::is_rang_guide_v2_message(&candidate, bot_user_id) {
                    message_ids.push(message_id);
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        message_ids.sort_unstable();
        message_ids.dedup();
        Ok(message_ids)
    }

    async fn fetch_regelwerk_v2_message_ids(&self, bot_user_id: u64) -> ServerSyncResult<Vec<u64>> {
        let mut before: Option<u64> = None;
        let mut message_ids = Vec::new();
        for _ in 0..5 {
            let mut url = format!(
                "{}/channels/{}/messages?limit=100",
                self.discord_api_base.trim_end_matches('/'),
                regelwerk_publish::REGELWERK_CHANNEL_ID
            );
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                let candidate = regelwerk_publish::RegelwerkV2Message {
                    message_id,
                    author_id,
                    flags: message.flags,
                    components: message.components.clone(),
                };
                if regelwerk_publish::is_regelwerk_v2_message(&candidate, bot_user_id) {
                    message_ids.push(message_id);
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        message_ids.sort_unstable();
        message_ids.dedup();
        Ok(message_ids)
    }

    async fn fetch_support_v2_message_ids(&self, bot_user_id: u64) -> ServerSyncResult<Vec<u64>> {
        let mut before: Option<u64> = None;
        let mut message_ids = Vec::new();
        for _ in 0..5 {
            let mut url = format!(
                "{}/channels/{}/messages?limit=100",
                self.discord_api_base.trim_end_matches('/'),
                support_publish::SUPPORT_CHANNEL_ID
            );
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                let candidate = support_publish::SupportV2Message {
                    message_id,
                    author_id,
                    flags: message.flags,
                    components: message.components.clone(),
                };
                if support_publish::is_support_v2_message(&candidate, bot_user_id) {
                    message_ids.push(message_id);
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        message_ids.sort_unstable();
        message_ids.dedup();
        Ok(message_ids)
    }

    async fn fetch_faq_v2_message_ids(&self, bot_user_id: u64) -> ServerSyncResult<Vec<u64>> {
        let mut before: Option<u64> = None;
        let mut message_ids = Vec::new();
        for _ in 0..5 {
            let mut url = format!(
                "{}/channels/{}/messages?limit=100",
                self.discord_api_base.trim_end_matches('/'),
                faq_publish::FAQ_CHANNEL_ID
            );
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                let candidate = faq_publish::FaqV2Message {
                    message_id,
                    author_id,
                    flags: message.flags,
                    components: message.components.clone(),
                };
                if faq_publish::is_faq_v2_message(&candidate, bot_user_id) {
                    message_ids.push(message_id);
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        message_ids.sort_unstable();
        message_ids.dedup();
        Ok(message_ids)
    }

    async fn fetch_regelwerk_legacy_cleanup_candidate(
        &self,
        bot_user_id: u64,
    ) -> ServerSyncResult<(
        Option<regelwerk_publish::RegelwerkLegacyCleanupCandidate>,
        Vec<String>,
    )> {
        let mut warnings = Vec::new();
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages/{}",
            regelwerk_publish::REGELWERK_CHANNEL_ID,
            regelwerk_publish::REGELWERK_OLD_MESSAGE_ID
        ));
        let response = self.discord_get_response(url).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            warnings.push(format!(
                "Regelwerk: alte Message-ID {} wurde nicht gefunden",
                regelwerk_publish::REGELWERK_OLD_MESSAGE_ID
            ));
            return Ok((None, warnings));
        }
        let message: DiscordMessage = discord_regelwerk_json_response(response, "GET")?;
        let message_id = parse_discord_id("Message-ID", &message.id)?;
        let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
        let candidate = regelwerk_publish::RegelwerkLegacyMessage {
            message_id,
            author_id,
            content: message.content,
        };
        if regelwerk_publish::is_legacy_regelwerk_cleanup_candidate(&candidate, bot_user_id) {
            Ok((
                Some(regelwerk_publish::RegelwerkLegacyCleanupCandidate {
                    message_id,
                    content_prefix: regelwerk_publish::REGELWERK_LEGACY_CONTENT_PREFIX.to_string(),
                }),
                warnings,
            ))
        } else {
            warnings.push(format!(
                "Regelwerk: alte Message-ID {message_id} passt nicht zur erwarteten Bot-/Content-Signatur und wird nicht geloescht"
            ));
            Ok((None, warnings))
        }
    }

    async fn delete_regelwerk_v2_messages(
        &self,
        stored_message_ids: &[u64],
        bot_user_id: u64,
    ) -> ServerSyncResult<StaticV2DeleteOutcome> {
        let mut message_ids = stored_message_ids.to_vec();
        message_ids.sort_unstable();
        message_ids.dedup();

        let mut deleted = Vec::new();
        let mut warnings = Vec::new();
        for message_id in message_ids {
            let fetch_url = self.discord_api_url(&format!(
                "/channels/{}/messages/{message_id}",
                regelwerk_publish::REGELWERK_CHANNEL_ID
            ));
            let response = match self.discord_get_response(fetch_url).await {
                Ok(response) => response,
                Err(err) => {
                    warnings.push(format!(
                        "Regelwerk: gespeicherte Message-ID {message_id} konnte vor Delete nicht gelesen werden: {err}"
                    ));
                    continue;
                }
            };
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                warnings.push(format!(
                    "Regelwerk: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
                continue;
            }
            let message: DiscordMessage = match discord_regelwerk_json_response(response, "GET") {
                Ok(message) => message,
                Err(err) => {
                    warnings.push(format!(
                        "Regelwerk: gespeicherte Message-ID {message_id} konnte vor Delete nicht validiert werden: {err}"
                    ));
                    continue;
                }
            };
            let author_id = match parse_discord_id("Message-Author-ID", &message.author.id) {
                Ok(author_id) => author_id,
                Err(err) => {
                    warnings.push(format!(
                        "Regelwerk: gespeicherte Message-ID {message_id} hat ungueltigen Autor und wird nicht geloescht: {err}"
                    ));
                    continue;
                }
            };
            let candidate = regelwerk_publish::RegelwerkV2Message {
                message_id,
                author_id,
                flags: message.flags,
                components: message.components,
            };
            if !regelwerk_publish::is_regelwerk_v2_message(&candidate, bot_user_id) {
                warnings.push(format!(
                    "Regelwerk: gespeicherte Message-ID {message_id} passt nicht zur eigenen V2-Signatur und wird nicht geloescht"
                ));
                continue;
            }

            if self
                .discord_delete(self.discord_api_url(&format!(
                    "/channels/{}/messages/{message_id}",
                    regelwerk_publish::REGELWERK_CHANNEL_ID
                )))
                .await?
            {
                deleted.push(message_id);
            } else {
                warnings.push(format!(
                    "Regelwerk: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
            }
        }
        Ok(StaticV2DeleteOutcome { deleted, warnings })
    }

    async fn delete_support_messages(
        &self,
        stored_message_ids: &[u64],
        bot_user_id: u64,
    ) -> ServerSyncResult<StaticV2DeleteOutcome> {
        let mut message_ids = stored_message_ids.to_vec();
        message_ids.sort_unstable();
        message_ids.dedup();

        let mut deleted = Vec::new();
        let mut warnings = Vec::new();
        for message_id in message_ids {
            let fetch_url = self.discord_api_url(&format!(
                "/channels/{}/messages/{message_id}",
                support_publish::SUPPORT_CHANNEL_ID
            ));
            let response = match self.discord_get_response(fetch_url).await {
                Ok(response) => response,
                Err(err) => {
                    warnings.push(format!(
                        "Support: gespeicherte Message-ID {message_id} konnte vor Delete nicht gelesen werden: {err}"
                    ));
                    continue;
                }
            };
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                warnings.push(format!(
                    "Support: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
                continue;
            }
            let message: DiscordMessage = match discord_regelwerk_json_response(response, "GET") {
                Ok(message) => message,
                Err(err) => {
                    warnings.push(format!(
                        "Support: gespeicherte Message-ID {message_id} konnte vor Delete nicht validiert werden: {err}"
                    ));
                    continue;
                }
            };
            let author_id = match parse_discord_id("Message-Author-ID", &message.author.id) {
                Ok(author_id) => author_id,
                Err(err) => {
                    warnings.push(format!(
                        "Support: gespeicherte Message-ID {message_id} hat ungueltigen Autor und wird nicht geloescht: {err}"
                    ));
                    continue;
                }
            };
            let candidate = support_publish::SupportV2Message {
                message_id,
                author_id,
                flags: message.flags,
                components: message.components,
            };
            if !support_publish::is_support_v2_message(&candidate, bot_user_id) {
                warnings.push(format!(
                    "Support: gespeicherte Message-ID {message_id} passt nicht zur eigenen V2-Signatur und wird nicht geloescht"
                ));
                continue;
            }

            if self
                .discord_delete(self.discord_api_url(&format!(
                    "/channels/{}/messages/{message_id}",
                    support_publish::SUPPORT_CHANNEL_ID
                )))
                .await?
            {
                deleted.push(message_id);
            } else {
                warnings.push(format!(
                    "Support: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
            }
        }
        Ok(StaticV2DeleteOutcome { deleted, warnings })
    }

    async fn cleanup_new_static_v2_posts_best_effort(
        &self,
        label: &'static str,
        channel_id: u64,
        message_ids: &[u64],
    ) {
        for &message_id in message_ids {
            match self
                .discord_delete(
                    self.discord_api_url(&format!("/channels/{channel_id}/messages/{message_id}")),
                )
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(
                        %err,
                        message_id,
                        label,
                        "Components-V2: neu gepostete Message konnte nach Teilfehler nicht bereinigt werden"
                    );
                }
            }
        }
    }

    async fn cleanup_regelwerk_legacy_message(
        &self,
        output: &mut regelwerk_publish::RegelwerkPublishOutput,
        bot_user_id: u64,
    ) -> ServerSyncResult<()> {
        let Some(candidate) = output.legacy_cleanup_candidate.clone() else {
            return Ok(());
        };
        let url = self.discord_api_url(&format!(
            "/channels/{}/messages/{}",
            regelwerk_publish::REGELWERK_CHANNEL_ID,
            candidate.message_id
        ));
        let response = self.discord_get_response(url).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            output.warnings.push(format!(
                "Regelwerk: Legacy-Message {} war beim Cleanup bereits weg",
                candidate.message_id
            ));
            return Ok(());
        }
        let message: DiscordMessage = discord_regelwerk_json_response(response, "GET")?;
        let message_id = parse_discord_id("Message-ID", &message.id)?;
        let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
        let legacy = regelwerk_publish::RegelwerkLegacyMessage {
            message_id,
            author_id,
            content: message.content,
        };
        if !regelwerk_publish::is_legacy_regelwerk_cleanup_candidate(&legacy, bot_user_id) {
            output.warnings.push(format!(
                "Regelwerk: Legacy-Message {message_id} wurde vor Delete nicht revalidiert und bleibt stehen"
            ));
            return Ok(());
        }
        if self
            .discord_delete(self.discord_api_url(&format!(
                "/channels/{}/messages/{message_id}",
                regelwerk_publish::REGELWERK_CHANNEL_ID
            )))
            .await?
        {
            output.deleted_legacy_message_ids.push(message_id);
        } else {
            output.warnings.push(format!(
                "Regelwerk: Legacy-Message {message_id} war beim Cleanup bereits weg"
            ));
        }
        Ok(())
    }

    async fn cleanup_rang_guide_legacy_messages(
        &self,
        output: &mut rang_guide_publish::RangGuidePublishOutput,
    ) -> ServerSyncResult<()> {
        let candidates = output.legacy_cleanup_candidates.clone();
        for candidate in candidates {
            match self
                .discord_delete(format!(
                    "{}/channels/{}/messages/{}",
                    self.discord_api_base.trim_end_matches('/'),
                    rang_guide_publish::RANG_GUIDE_CHANNEL_ID,
                    candidate.message_id
                ))
                .await
            {
                Ok(true) => output.deleted_legacy_message_ids.push(candidate.message_id),
                Ok(false) => output.warnings.push(format!(
                    "Rang-Guide: Legacy-Message {} war beim Cleanup bereits weg",
                    candidate.message_id
                )),
                Err(err) => output.warnings.push(format!(
                    "Rang-Guide: Legacy-Message {} konnte nicht geloescht werden: {err}",
                    candidate.message_id
                )),
            }
        }
        Ok(())
    }

    async fn delete_welcome_messages(
        &self,
        channel_id: u64,
        stored_message_ids: &BTreeMap<String, Vec<u64>>,
        discovered_message_ids: &BTreeMap<String, Vec<u64>>,
    ) -> ServerSyncResult<Vec<u64>> {
        let mut message_ids = stored_message_ids
            .values()
            .chain(discovered_message_ids.values())
            .flat_map(|ids| ids.iter().copied())
            .collect::<Vec<_>>();
        message_ids.sort_unstable();
        message_ids.dedup();

        let mut deleted = Vec::new();
        for message_id in message_ids {
            if self
                .discord_delete(format!(
                    "{DISCORD_API_BASE}/channels/{channel_id}/messages/{message_id}"
                ))
                .await?
            {
                deleted.push(message_id);
            }
        }
        Ok(deleted)
    }

    async fn fetch_welcome_marker_message_ids(
        &self,
        channel_id: u64,
        bot_user_id: u64,
    ) -> ServerSyncResult<BTreeMap<String, Vec<u64>>> {
        let mut messages = BTreeMap::new();
        let mut before: Option<u64> = None;
        loop {
            let mut url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages?limit=100");
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                if author_id != bot_user_id {
                    continue;
                }
                for embed in &message.embeds {
                    if let Some(marker) = embed
                        .footer
                        .as_ref()
                        .and_then(|footer| footer.text.as_deref())
                    {
                        if let Some(section_id) =
                            welcome_publish::welcome_section_id_from_marker(marker)
                        {
                            messages
                                .entry(section_id.to_string())
                                .or_insert_with(Vec::new)
                                .push(message_id);
                            continue;
                        }
                    }
                    if let Some(section_id) = embed
                        .title
                        .as_deref()
                        .and_then(welcome_publish::welcome_section_id_from_title)
                    {
                        messages
                            .entry(section_id.to_string())
                            .or_insert_with(Vec::new)
                            .push(message_id);
                    }
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        for ids in messages.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        Ok(messages)
    }

    async fn edit_welcome_message(
        &self,
        channel_id: u64,
        message_id: u64,
        section: &welcome_publish::WelcomeSectionOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages/{message_id}");
        let response = self
            .send_welcome_message_payload("PATCH", url, &section.payload)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let message: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &message.id).map(Some)
    }

    async fn post_welcome_message(
        &self,
        channel_id: u64,
        section: &welcome_publish::WelcomeSectionOutput,
    ) -> ServerSyncResult<u64> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages");
        let response = self
            .send_welcome_message_payload("POST", url, &section.payload)
            .await?;
        let message: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Message-ID", &message.id)
    }

    async fn send_welcome_message_payload(
        &self,
        method: &'static str,
        url: String,
        payload: &welcome_publish::WelcomeMessagePayload,
    ) -> ServerSyncResult<DiscordRestResponse> {
        let payload_value = serde_json::to_value(payload)?;
        if !payload.attachments.is_empty() {
            let payload_text = serde_json::to_string(&payload_value)?;
            let mut files = Vec::new();
            for attachment in &payload.attachments {
                let path = welcome_publish::welcome_repo_root().join(&attachment.relative_path);
                let bytes = std::fs::read(&path).map_err(|err| {
                    ServerSyncError::internal(format!(
                        "Welcome-Attachment `{}` konnte nicht gelesen werden: {err}",
                        path.display()
                    ))
                })?;
                files.push((attachment.id, attachment.filename.clone(), bytes));
            }
            return discord_regelwerk_send_with_retry(
                || {
                    let request = match method {
                        "POST" => self.http_client.post(url.clone()),
                        "PATCH" => self.http_client.patch(url.clone()),
                        other => unreachable!("unsupported Discord method {other}"),
                    }
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON);
                    let payload_text = payload_text.clone();
                    let files = files.clone();
                    async move {
                        let mut form =
                            reqwest::multipart::Form::new().text("payload_json", payload_text);
                        for (id, filename, bytes) in files {
                            let part = reqwest::multipart::Part::bytes(bytes)
                                .file_name(filename)
                                .mime_str("image/png")?;
                            form = form.part(format!("files[{id}]"), part);
                        }
                        let response = request.multipart(form).send().await?;
                        DiscordRestResponse::from_response(response).await
                    }
                },
                method,
            )
            .await;
        }

        discord_regelwerk_send_with_retry(
            || {
                let request = match method {
                    "POST" => self.http_client.post(url.clone()),
                    "PATCH" => self.http_client.patch(url.clone()),
                    other => unreachable!("unsupported Discord method {other}"),
                }
                .header("Authorization", format!("Bot {}", self.discord_token))
                .header("Content-Type", "application/json")
                .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                .json(&payload_value);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            method,
        )
        .await
    }

    async fn load_rollback_artifact(
        &self,
        rollback_export_id: Option<i64>,
    ) -> ServerSyncResult<(i64, String, RollbackArtifact)> {
        let guild_id = id_to_i64(self.guild_id)?;
        let row = if let Some(rollback_export_id) = rollback_export_id {
            sqlx::query(
                "SELECT rollback_export_id, artifact_hash, artifact_json::text AS artifact_json
                   FROM server_config.rollback_exports
                  WHERE rollback_export_id = $1
                    AND guild_id = $2
                    AND expires_at > now()",
            )
            .bind(rollback_export_id)
            .bind(guild_id)
            .fetch_optional(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT rollback_export_id, artifact_hash, artifact_json::text AS artifact_json
                   FROM server_config.rollback_exports
                  WHERE guild_id = $1
                    AND expires_at > now()
                  ORDER BY created_at DESC, rollback_export_id DESC
                  LIMIT 1",
            )
            .bind(guild_id)
            .fetch_optional(&self.pool)
            .await?
        }
        .ok_or_else(|| {
            ServerSyncError::bad_request(match rollback_export_id {
                Some(id) => format!("Rollback-Export {id} nicht gefunden oder abgelaufen"),
                None => "Kein gueltiger Rollback-Export gefunden".to_string(),
            })
        })?;

        let loaded_id: i64 = row.try_get("rollback_export_id")?;
        let stored_hash: String = row.try_get("artifact_hash")?;
        let artifact_text: String = row.try_get("artifact_json")?;
        let artifact_value: Value = serde_json::from_str(&artifact_text).map_err(|err| {
            ServerSyncError::bad_request(format!(
                "Rollback-Artefakt {loaded_id} ist kein gueltiges JSON: {err}"
            ))
        })?;

        let artifact =
            verify_rollback_artifact(loaded_id, &stored_hash, artifact_value, self.guild_id)?;
        Ok((loaded_id, stored_hash, artifact))
    }
}

fn adopt_regelwerk_message_ids(
    output: &mut regelwerk_publish::RegelwerkPublishOutput,
    discovered_message_ids: &[u64],
    confirm: bool,
) {
    output.stored_message_ids = discovered_message_ids.to_vec();
    output.repost_required = false;
    output.warnings.push(format!(
        "Regelwerk: KV-Message-IDs fehlen oder sind unvollstaendig; vorhandene eigene V2-Messages werden adoptiert: {:?}",
        discovered_message_ids
    ));
    for (message_index, message_id) in discovered_message_ids.iter().copied().enumerate() {
        if let Some(message) = output.messages.get_mut(message_index) {
            message.stored_message_id = Some(message_id);
            message.message_id = Some(message_id);
            if !confirm {
                message.action = "planned_adopted_edit".to_string();
            }
        }
    }
}

fn adopt_support_message_ids(
    output: &mut support_publish::SupportPublishOutput,
    discovered_message_ids: &[u64],
    confirm: bool,
) {
    output.stored_message_ids = discovered_message_ids.to_vec();
    output.repost_required = false;
    output.warnings.push(format!(
        "Support: KV-Message-IDs fehlen oder sind unvollstaendig; vorhandene eigene V2-Messages werden adoptiert: {:?}",
        discovered_message_ids
    ));
    for (message_index, message_id) in discovered_message_ids.iter().copied().enumerate() {
        if let Some(message) = output.messages.get_mut(message_index) {
            message.stored_message_id = Some(message_id);
            message.message_id = Some(message_id);
            if !confirm {
                message.action = "planned_adopted_edit".to_string();
            }
        }
    }
}

fn adopt_faq_message_ids(
    output: &mut faq_publish::FaqPublishOutput,
    discovered_message_ids: &[u64],
    confirm: bool,
) {
    output.stored_message_ids = discovered_message_ids.to_vec();
    output.repost_required = false;
    output.warnings.push(format!(
        "FAQ: KV-Message-IDs fehlen oder sind unvollstaendig; vorhandene eigene V2-Messages werden adoptiert: {:?}",
        discovered_message_ids
    ));
    for (message_index, message_id) in discovered_message_ids.iter().copied().enumerate() {
        if let Some(message) = output.messages.get_mut(message_index) {
            message.stored_message_id = Some(message_id);
            message.message_id = Some(message_id);
            if !confirm {
                message.action = "planned_adopted_edit".to_string();
            }
        }
    }
}

#[async_trait]
impl ServerSyncOps for ServerSyncService {
    fn guild_id(&self) -> u64 {
        self.guild_id
    }

    async fn snapshot(
        &self,
        _requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<SnapshotOutput> {
        let report = dl_server_as_code::import_live_guild_snapshot(
            &self.pool,
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        Ok(snapshot_output(report))
    }

    async fn rollback_export(
        &self,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<RollbackExportOutput> {
        let model = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let report =
            dl_server_as_code::db::persist_snapshot_model(&self.pool, &model, "rollback_export")
                .await?;
        let archive_enabled = self.welle2b_archive_enabled().await?;
        let derivation = dl_server_as_code::derive_desired_model_with_options(
            &model,
            dl_server_as_code::DesiredModelOptions {
                welle2b_archive_enabled: archive_enabled,
                ..dl_server_as_code::DesiredModelOptions::default()
            },
        )?;
        let member_role_assignments = self.fetch_member_role_assignments().await?;
        let native_onboarding_config = self.fetch_native_onboarding_config().await?;
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let artifact = RollbackArtifact {
            version: ROLLBACK_VERSION.to_string(),
            guild_id: self.guild_id,
            created_at,
            snapshot_id: report.snapshot_id,
            structure_snapshot: RollbackGuildModel::from_model(&model),
            dynamic_namespaces: derivation.dynamic_namespaces,
            documented_exceptions: derivation.exceptions,
            member_role_assignments,
            native_onboarding_config,
        };
        let artifact_value = serde_json::to_value(&artifact)?;
        let artifact_text = serde_json::to_string_pretty(&artifact_value)?;
        let artifact_hash = sha256_json_value(&artifact_value)?;
        let metadata = json!({
            "version": ROLLBACK_VERSION,
            "member_count": artifact.member_role_assignments.len(),
            "dynamic_namespace_count": artifact.dynamic_namespaces.len(),
            "documented_exception_count": artifact.documented_exceptions.len(),
        });
        let rollback_export_id: i64 = sqlx::query_scalar(
            "INSERT INTO server_config.rollback_exports
             (guild_id, snapshot_id, created_by_user_id, artifact_hash, artifact_json, metadata)
             VALUES ($1, $2, $3, $4, $5::text::jsonb, $6::text::jsonb)
             RETURNING rollback_export_id",
        )
        .bind(id_to_i64(self.guild_id)?)
        .bind(report.snapshot_id)
        .bind(requested_by_user_id.map(id_to_i64).transpose()?)
        .bind(&artifact_hash)
        .bind(artifact_value.to_string())
        .bind(metadata.to_string())
        .fetch_one(&self.pool)
        .await?;

        let member_role_edges = artifact
            .member_role_assignments
            .iter()
            .map(|entry| entry.role_ids.len())
            .sum();
        let filename = rollback_filename(rollback_export_id, report.snapshot_id);
        Ok(RollbackExportOutput {
            rollback_export_id,
            snapshot_id: report.snapshot_id,
            guild_id: self.guild_id,
            artifact_hash,
            captured_at: report
                .captured_at
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            members: artifact.member_role_assignments.len(),
            member_role_edges,
            filename,
            artifact_text,
            artifact: artifact_value,
        })
    }

    async fn diff(&self, requested_by_user_id: Option<u64>) -> ServerSyncResult<DiffOutput> {
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let snapshot =
            dl_server_as_code::db::persist_snapshot_model(&self.pool, &live, "diff_preview")
                .await?;
        let archive_enabled = self.welle2b_archive_enabled().await?;
        let derivation = dl_server_as_code::derive_desired_model_with_options(
            &live,
            dl_server_as_code::DesiredModelOptions {
                welle2b_archive_enabled: archive_enabled,
                ..dl_server_as_code::DesiredModelOptions::default()
            },
        )?;
        dl_server_as_code::persist_desired_model(
            &self.pool,
            &derivation.desired,
            &derivation.dynamic_namespaces,
            &derivation.exceptions,
        )
        .await?;
        let diff = dl_server_as_code::diff_models_with_options(
            &derivation.desired,
            &live,
            &derivation.dynamic_namespaces,
            &derivation.exceptions,
            dl_server_as_code::DiffOptions {
                compare_relative_positions: false,
            },
        )?;
        let preview = dl_server_as_code::persist_diff_preview(
            &self.pool,
            Some(snapshot.snapshot_id),
            &diff,
            requested_by_user_id,
        )
        .await?;
        Ok(DiffOutput {
            preview_id: preview.preview_id,
            guild_id: preview.guild_id,
            diff_hash: preview.diff_hash,
            human_summary: preview.human_summary,
            warnings: derivation.warnings,
            diff_text: serde_json::to_string_pretty(&preview.diff)?,
        })
    }

    async fn restore(
        &self,
        rollback_export_id: Option<i64>,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<RestoreOutput> {
        let (rollback_export_id, artifact_hash, artifact) =
            self.load_rollback_artifact(rollback_export_id).await?;
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let snapshot =
            dl_server_as_code::db::persist_snapshot_model(&self.pool, &live, "restore_preview")
                .await?;
        let rollback_model = artifact.structure_snapshot.to_model()?;
        let diff = dl_server_as_code::diff_models_with_options(
            &rollback_model,
            &live,
            &artifact.dynamic_namespaces,
            &artifact.documented_exceptions,
            dl_server_as_code::DiffOptions {
                compare_relative_positions: false,
            },
        )?;
        let preview = dl_server_as_code::persist_diff_preview(
            &self.pool,
            Some(snapshot.snapshot_id),
            &diff,
            requested_by_user_id,
        )
        .await?;
        Ok(RestoreOutput {
            rollback_export_id,
            preview_id: preview.preview_id,
            guild_id: preview.guild_id,
            artifact_hash,
            diff_hash: preview.diff_hash,
            human_summary: preview.human_summary,
            warnings: vec![
                "Member-Rollen und natives Onboarding bleiben im Artefakt, werden in Welle 2a aber nicht restored.".to_string(),
            ],
            diff_text: serde_json::to_string_pretty(&preview.diff)?,
        })
    }

    async fn apply(
        &self,
        preview_id: i64,
        hash: String,
        confirm: bool,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ApplyOutput> {
        if hash.trim().is_empty() {
            return Err(ServerSyncError::bad_request("hash fehlt"));
        }
        let report = dl_server_as_code::apply_preview(
            &self.pool,
            preview_id,
            hash.trim(),
            ApplyOptions {
                dry_run: !confirm,
                requested_by_user_id,
            },
            self.adapter.http.as_ref(),
            AUDIT_LOG_REASON,
        )
        .await?;
        apply_output(report)
    }

    async fn onboarding_preview(
        &self,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<OnboardingPreviewOutput> {
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let snapshot =
            dl_server_as_code::db::persist_snapshot_model(&self.pool, &live, "onboarding_preview")
                .await?;
        let live_onboarding = self.fetch_native_onboarding_config().await?;
        let built = build_welle2b_onboarding_config(&live_onboarding, &live)?;
        if !built.blockers.is_empty() {
            return Ok(OnboardingPreviewOutput {
                preview_id: None,
                guild_id: self.guild_id,
                diff_hash: None,
                human_summary: onboarding_blocker_summary(&built.blockers),
                blockers: built.blockers,
                warnings: built.warnings,
                diff_text: "{}".to_string(),
                desired_config: Some(serde_json::to_value(&built.config)?),
            });
        }

        let diff = onboarding_diff(self.guild_id, &live_onboarding, &built.config)?;
        let human_summary = onboarding_human_summary(&diff, &built.config);
        let preview = persist_onboarding_preview(
            &self.pool,
            Some(snapshot.snapshot_id),
            &diff,
            &human_summary,
            requested_by_user_id,
        )
        .await?;
        Ok(OnboardingPreviewOutput {
            preview_id: Some(preview.preview_id),
            guild_id: self.guild_id,
            diff_hash: Some(preview.diff_hash),
            human_summary,
            blockers: Vec::new(),
            warnings: built.warnings,
            diff_text: serde_json::to_string_pretty(&diff)?,
            desired_config: Some(serde_json::to_value(&built.config)?),
        })
    }

    async fn onboarding_apply(
        &self,
        preview_id: i64,
        hash: String,
        confirm: bool,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ApplyOutput> {
        if hash.trim().is_empty() {
            return Err(ServerSyncError::bad_request("hash fehlt"));
        }
        let preview = load_onboarding_preview(&self.pool, preview_id, self.guild_id).await?;
        let confirmed = hash.trim();
        if preview.diff_hash != confirmed {
            let apply_run_id = insert_onboarding_apply_run(OnboardingApplyRunInsert {
                pool: &self.pool,
                preview_id,
                guild_id: preview.guild_id,
                confirmed_hash: confirmed,
                requested_by_user_id,
                dry_run: confirm,
                status: "hash_mismatch",
                result: json!({
                    "stored": preview.diff_hash,
                    "confirmed": confirmed,
                }),
                error_text: Some("diff hash binding mismatch"),
            })
            .await?;
            return Err(ServerSyncError::bad_request(format!(
                "Diff-Hash stimmt nicht: apply_run_id {apply_run_id}, erwartet {}, bekommen {}",
                preview.diff_hash, confirmed
            )));
        }

        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        // Keine 7/5-Revalidierung mehr: Die Defaults sind 1:1 der Owner-Zustand
        // (Discord haelt sie bereits); ein echter PUT-Fehler wird ehrlich gemeldet.
        validate_onboarding_references(&live, &preview.config)
            .map_err(ServerSyncError::bad_request)?;

        if !confirm {
            let details = json!({
                "preview_id": preview_id,
                "dry_run": true,
                "planned_changes": 1,
                "onboarding": preview.config,
            });
            let apply_run_id = insert_onboarding_apply_run(OnboardingApplyRunInsert {
                pool: &self.pool,
                preview_id,
                guild_id: preview.guild_id,
                confirmed_hash: confirmed,
                requested_by_user_id,
                dry_run: true,
                status: "dry_run",
                result: details.clone(),
                error_text: None,
            })
            .await?;
            return Ok(ApplyOutput {
                apply_run_id,
                preview_id,
                dry_run: true,
                applied: 0,
                skipped: 1,
                failed: 0,
                details_text: serde_json::to_string_pretty(&details)?,
                details,
            });
        }

        let apply_run_id = insert_onboarding_apply_run(OnboardingApplyRunInsert {
            pool: &self.pool,
            preview_id,
            guild_id: preview.guild_id,
            confirmed_hash: confirmed,
            requested_by_user_id,
            dry_run: false,
            status: "running",
            result: json!({}),
            error_text: None,
        })
        .await?;
        if let Err(err) = self.put_native_onboarding_config(&preview.config).await {
            let details = json!({
                "preview_id": preview_id,
                "applied": 0,
            });
            finish_onboarding_apply_run(
                &self.pool,
                apply_run_id,
                "failed",
                details,
                Some(&err.to_string()),
            )
            .await?;
            return Err(err);
        }

        let details = json!({
            "preview_id": preview_id,
            "dry_run": false,
            "applied": 1,
        });
        finish_onboarding_apply_run(&self.pool, apply_run_id, "applied", details.clone(), None)
            .await?;
        sqlx::query(
            "UPDATE server_config.diff_previews SET applied_at = now() WHERE preview_id = $1",
        )
        .bind(preview_id)
        .execute(&self.pool)
        .await?;
        Ok(ApplyOutput {
            apply_run_id,
            preview_id,
            dry_run: false,
            applied: 1,
            skipped: 0,
            failed: 0,
            details_text: serde_json::to_string_pretty(&details)?,
            details,
        })
    }

    async fn serverguide_preview(
        &self,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ServerGuidePreviewOutput> {
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let snapshot =
            dl_server_as_code::db::persist_snapshot_model(&self.pool, &live, "serverguide_preview")
                .await?;
        let live_serverguide = self.fetch_server_guide_config().await?;
        let built = build_server_guide_config(&live_serverguide, &live);
        let Some(config) = built.config else {
            return Ok(ServerGuidePreviewOutput {
                preview_id: None,
                guild_id: self.guild_id,
                diff_hash: None,
                human_summary: serverguide_blocker_summary(&built.blockers),
                blockers: built.blockers,
                warnings: built.warnings,
                diff_text: "{}".to_string(),
                desired_config: None,
            });
        };

        let diff = serverguide_diff(self.guild_id, &live_serverguide, &config)?;
        let human_summary = serverguide_human_summary(&diff, &config);
        let preview = persist_onboarding_preview(
            &self.pool,
            Some(snapshot.snapshot_id),
            &diff,
            &human_summary,
            requested_by_user_id,
        )
        .await?;
        Ok(ServerGuidePreviewOutput {
            preview_id: Some(preview.preview_id),
            guild_id: self.guild_id,
            diff_hash: Some(preview.diff_hash),
            human_summary,
            blockers: Vec::new(),
            warnings: built.warnings,
            diff_text: serde_json::to_string_pretty(&diff)?,
            desired_config: Some(serde_json::to_value(&config)?),
        })
    }

    async fn serverguide_apply(
        &self,
        preview_id: i64,
        hash: String,
        confirm: bool,
        requested_by_user_id: Option<u64>,
    ) -> ServerSyncResult<ApplyOutput> {
        if hash.trim().is_empty() {
            return Err(ServerSyncError::bad_request("hash fehlt"));
        }
        let preview = load_serverguide_preview(&self.pool, preview_id, self.guild_id).await?;
        let confirmed = hash.trim();
        if preview.diff_hash != confirmed {
            let apply_run_id = insert_onboarding_apply_run(OnboardingApplyRunInsert {
                pool: &self.pool,
                preview_id,
                guild_id: preview.guild_id,
                confirmed_hash: confirmed,
                requested_by_user_id,
                dry_run: confirm,
                status: "hash_mismatch",
                result: json!({
                    "stored": preview.diff_hash,
                    "confirmed": confirmed,
                }),
                error_text: Some("diff hash binding mismatch"),
            })
            .await?;
            return Err(ServerSyncError::bad_request(format!(
                "Diff-Hash stimmt nicht: apply_run_id {apply_run_id}, erwartet {}, bekommen {}",
                preview.diff_hash, confirmed
            )));
        }

        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        validate_server_guide_config(&live, &preview.config)
            .map_err(ServerSyncError::bad_request)?;

        if !confirm {
            let details = json!({
                "preview_id": preview_id,
                "dry_run": true,
                "planned_changes": 1,
                "server_guide": preview.config,
            });
            let apply_run_id = insert_onboarding_apply_run(OnboardingApplyRunInsert {
                pool: &self.pool,
                preview_id,
                guild_id: preview.guild_id,
                confirmed_hash: confirmed,
                requested_by_user_id,
                dry_run: true,
                status: "dry_run",
                result: details.clone(),
                error_text: None,
            })
            .await?;
            return Ok(ApplyOutput {
                apply_run_id,
                preview_id,
                dry_run: true,
                applied: 0,
                skipped: 1,
                failed: 0,
                details_text: serde_json::to_string_pretty(&details)?,
                details,
            });
        }

        let apply_run_id = insert_onboarding_apply_run(OnboardingApplyRunInsert {
            pool: &self.pool,
            preview_id,
            guild_id: preview.guild_id,
            confirmed_hash: confirmed,
            requested_by_user_id,
            dry_run: false,
            status: "running",
            result: json!({}),
            error_text: None,
        })
        .await?;
        if let Err(err) = self.put_server_guide_config(&preview.config).await {
            let details = json!({
                "preview_id": preview_id,
                "applied": 0,
            });
            finish_onboarding_apply_run(
                &self.pool,
                apply_run_id,
                "failed",
                details,
                Some(&err.to_string()),
            )
            .await?;
            return Err(err);
        }

        let details = json!({
            "preview_id": preview_id,
            "dry_run": false,
            "applied": 1,
        });
        finish_onboarding_apply_run(&self.pool, apply_run_id, "applied", details.clone(), None)
            .await?;
        sqlx::query(
            "UPDATE server_config.diff_previews SET applied_at = now() WHERE preview_id = $1",
        )
        .bind(preview_id)
        .execute(&self.pool)
        .await?;
        Ok(ApplyOutput {
            apply_run_id,
            preview_id,
            dry_run: false,
            applied: 1,
            skipped: 0,
            failed: 0,
            details_text: serde_json::to_string_pretty(&details)?,
            details,
        })
    }

    async fn archive_enable(&self) -> ServerSyncResult<ArchiveFlagOutput> {
        self.set_welle2b_archive_enabled(true).await
    }

    async fn archive_disable(&self) -> ServerSyncResult<ArchiveFlagOutput> {
        self.set_welle2b_archive_enabled(false).await
    }

    async fn regelwerk_publish(&self, confirm: bool) -> ServerSyncResult<RegelwerkPublishOutput> {
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let content = build_regelwerk_text(&live)?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;
        let stored_message_id = self.load_regelwerk_message_id().await?;
        let thread_ids = self.fetch_regelwerk_threads().await?;
        let bot_messages = self
            .fetch_regelwerk_bot_messages(bot_user_id, stored_message_id)
            .await?;

        if !confirm {
            return Ok(regelwerk_output(RegelwerkOutputInput {
                dry_run: true,
                thread_count: thread_ids.len(),
                threads_deleted: 0,
                bot_messages: &bot_messages,
                bot_messages_deleted: 0,
                stored_message_id,
                posted_message_id: None,
                edited_message_id: None,
            }));
        }

        let mut threads_deleted = 0usize;
        for (index, thread_id) in thread_ids.iter().enumerate() {
            if self
                .discord_delete(format!("{DISCORD_API_BASE}/channels/{thread_id}"))
                .await?
            {
                threads_deleted += 1;
            }
            if index + 1 < thread_ids.len() {
                tokio::time::sleep(REGELWERK_DELETE_DELAY).await;
            }
        }

        let mut bot_messages_deleted = 0usize;
        for (index, message) in bot_messages.iter().enumerate() {
            if self
                .discord_delete(format!(
                    "{DISCORD_API_BASE}/channels/{RULES_CHANNEL_ID}/messages/{}",
                    message.message_id
                ))
                .await?
            {
                bot_messages_deleted += 1;
            }
            if index + 1 < bot_messages.len() {
                tokio::time::sleep(REGELWERK_DELETE_DELAY).await;
            }
        }

        let (posted_message_id, edited_message_id) = if let Some(message_id) = stored_message_id {
            match self.edit_regelwerk_message(message_id, &content).await? {
                Some(edited_id) => (None, Some(edited_id)),
                None => {
                    let posted_id = self.post_regelwerk_message(&content).await?;
                    self.store_regelwerk_message_id(posted_id).await?;
                    (Some(posted_id), None)
                }
            }
        } else {
            let posted_id = self.post_regelwerk_message(&content).await?;
            self.store_regelwerk_message_id(posted_id).await?;
            (Some(posted_id), None)
        };

        Ok(regelwerk_output(RegelwerkOutputInput {
            dry_run: false,
            thread_count: thread_ids.len(),
            threads_deleted,
            bot_messages: &bot_messages,
            bot_messages_deleted,
            stored_message_id,
            posted_message_id,
            edited_message_id,
        }))
    }

    async fn regelwerk_apply(
        &self,
        confirm: bool,
    ) -> ServerSyncResult<regelwerk_publish::RegelwerkPublishOutput> {
        let stored_message_ids = self
            .load_static_v2_message_ids(regelwerk_publish::REGELWERK_MESSAGE_ID_PREFIX)
            .await?;
        let stored_payload_format = self
            .load_serversync_kv(regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY)
            .await?;
        let stored_payload_hash = self
            .load_serversync_kv(regelwerk_publish::REGELWERK_PAYLOAD_HASH_KEY)
            .await?;
        let mut output = regelwerk_publish::build_regelwerk_publish_output(
            &self.rang_guide_repo_root,
            &stored_message_ids,
            stored_payload_format.as_deref(),
            stored_payload_hash.as_deref(),
            !confirm,
        )
        .map_err(ServerSyncError::bad_request)?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;
        let (legacy_candidate, legacy_warnings) = self
            .fetch_regelwerk_legacy_cleanup_candidate(bot_user_id)
            .await?;
        output.legacy_cleanup_candidate = legacy_candidate;
        output.warnings.extend(legacy_warnings);

        let discovered_v2_message_ids = self.fetch_regelwerk_v2_message_ids(bot_user_id).await?;
        let expected_message_count = output.messages.len();
        let mut adopted_from_history = false;
        if stored_message_ids.len() != expected_message_count {
            match discovered_v2_message_ids.len() {
                0 => {}
                count if count == expected_message_count => {
                    adopt_regelwerk_message_ids(&mut output, &discovered_v2_message_ids, confirm);
                    adopted_from_history = true;
                }
                count => {
                    return Err(ServerSyncError::bad_request(format!(
                        "Regelwerk: KV-Message-IDs fehlen/unvollstaendig, aber History-Scan fand {count} eigene V2-Messages fuer {expected_message_count} erwartete Messages; Apply blockiert gegen Duplikate."
                    )));
                }
            }
        }

        if !confirm {
            return Ok(output);
        }
        for warning in &output.warnings {
            tracing::warn!(%warning, "Regelwerk-Publish-Warnung");
        }

        if output.repost_required {
            let mut posted_message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let post_result = self
                    .post_regelwerk_v2_message(&output.messages[message_index])
                    .await;
                let message_id = match post_result {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        self.cleanup_new_static_v2_posts_best_effort(
                            "Regelwerk",
                            regelwerk_publish::REGELWERK_CHANNEL_ID,
                            &posted_message_ids,
                        )
                        .await;
                        return Err(err);
                    }
                };
                posted_message_ids.push(message_id);
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if let Err(err) = self
                .replace_static_v2_message_ids_and_metadata(
                    regelwerk_publish::REGELWERK_MESSAGE_ID_PREFIX,
                    regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY,
                    regelwerk_publish::REGELWERK_PAYLOAD_FORMAT,
                    regelwerk_publish::REGELWERK_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                    &posted_message_ids,
                )
                .await
            {
                self.cleanup_new_static_v2_posts_best_effort(
                    "Regelwerk",
                    regelwerk_publish::REGELWERK_CHANNEL_ID,
                    &posted_message_ids,
                )
                .await;
                return Err(err);
            }
            let delete_outcome = self
                .delete_regelwerk_v2_messages(&stored_message_ids, bot_user_id)
                .await?;
            output.deleted_message_ids = delete_outcome.deleted;
            output.warnings.extend(delete_outcome.warnings);
        } else if !adopted_from_history
            && regelwerk_publish::regelwerk_payload_is_unchanged(&output)
        {
            for message in &mut output.messages {
                if let Some(message_id) = message.stored_message_id {
                    message.action = "no_op".to_string();
                    message.message_id = Some(message_id);
                }
            }
            self.store_static_v2_metadata(
                regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY,
                regelwerk_publish::REGELWERK_PAYLOAD_FORMAT,
                regelwerk_publish::REGELWERK_PAYLOAD_HASH_KEY,
                &output.payload_hash,
            )
            .await?;
        } else {
            let mut message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let stored_message_id = output.messages[message_index].stored_message_id;
                if let Some(stored_message_id) = stored_message_id {
                    if let Some(message_id) = self
                        .edit_regelwerk_v2_message(
                            stored_message_id,
                            &output.messages[message_index],
                        )
                        .await?
                    {
                        message_ids.push(message_id);
                        output.messages[message_index].action = if adopted_from_history {
                            "adopted_edit".to_string()
                        } else {
                            "edited".to_string()
                        };
                        output.messages[message_index].message_id = Some(message_id);
                        output.messages[message_index].stored_message_id = Some(message_id);
                        output.edited_message_ids.push(message_id);
                        continue;
                    }
                    output.warnings.push(format!(
                        "Regelwerk: gespeicherte Message-ID {stored_message_id} ist stale (404); es wird neu gepostet."
                    ));
                }

                let message_id = self
                    .post_regelwerk_v2_message(&output.messages[message_index])
                    .await?;
                message_ids.push(message_id);
                self.store_static_v2_message_id(
                    regelwerk_publish::REGELWERK_MESSAGE_ID_PREFIX,
                    message_index,
                    message_id,
                )
                .await?;
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if adopted_from_history {
                self.replace_static_v2_message_ids_and_metadata(
                    regelwerk_publish::REGELWERK_MESSAGE_ID_PREFIX,
                    regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY,
                    regelwerk_publish::REGELWERK_PAYLOAD_FORMAT,
                    regelwerk_publish::REGELWERK_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                    &message_ids,
                )
                .await?;
            } else {
                self.store_static_v2_metadata(
                    regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY,
                    regelwerk_publish::REGELWERK_PAYLOAD_FORMAT,
                    regelwerk_publish::REGELWERK_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                )
                .await?;
            }
        }

        self.cleanup_regelwerk_legacy_message(&mut output, bot_user_id)
            .await?;
        output.dry_run = false;
        output.repost_required = false;
        Ok(output)
    }

    async fn support_apply(&self, confirm: bool) -> ServerSyncResult<SupportPublishOutput> {
        let stored_message_ids = self
            .load_static_v2_message_ids(support_publish::SUPPORT_MESSAGE_ID_PREFIX)
            .await?;
        let stored_payload_format = self
            .load_serversync_kv(support_publish::SUPPORT_PAYLOAD_FORMAT_KEY)
            .await?;
        let stored_payload_hash = self
            .load_serversync_kv(support_publish::SUPPORT_PAYLOAD_HASH_KEY)
            .await?;
        let mut output = support_publish::build_support_publish_output(
            &self.rang_guide_repo_root,
            &stored_message_ids,
            stored_payload_format.as_deref(),
            stored_payload_hash.as_deref(),
            !confirm,
        )
        .map_err(ServerSyncError::bad_request)?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;

        let discovered_v2_message_ids = self.fetch_support_v2_message_ids(bot_user_id).await?;
        let expected_message_count = output.messages.len();
        let mut adopted_from_history = false;
        if stored_message_ids.len() != expected_message_count {
            match discovered_v2_message_ids.len() {
                0 => {}
                count if count == expected_message_count => {
                    adopt_support_message_ids(&mut output, &discovered_v2_message_ids, confirm);
                    adopted_from_history = true;
                }
                count => {
                    return Err(ServerSyncError::bad_request(format!(
                        "Support: KV-Message-IDs fehlen/unvollstaendig, aber History-Scan fand {count} eigene V2-Messages fuer {expected_message_count} erwartete Messages; Apply blockiert gegen Duplikate."
                    )));
                }
            }
        }

        if !confirm {
            return Ok(output);
        }
        for warning in &output.warnings {
            tracing::warn!(%warning, "Support-Publish-Warnung");
        }

        if output.repost_required {
            let mut posted_message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let post_result = self
                    .post_support_message(&output.messages[message_index])
                    .await;
                let message_id = match post_result {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        self.cleanup_new_static_v2_posts_best_effort(
                            "Support",
                            support_publish::SUPPORT_CHANNEL_ID,
                            &posted_message_ids,
                        )
                        .await;
                        return Err(err);
                    }
                };
                posted_message_ids.push(message_id);
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if let Err(err) = self
                .replace_static_v2_message_ids_and_metadata(
                    support_publish::SUPPORT_MESSAGE_ID_PREFIX,
                    support_publish::SUPPORT_PAYLOAD_FORMAT_KEY,
                    support_publish::SUPPORT_PAYLOAD_FORMAT,
                    support_publish::SUPPORT_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                    &posted_message_ids,
                )
                .await
            {
                self.cleanup_new_static_v2_posts_best_effort(
                    "Support",
                    support_publish::SUPPORT_CHANNEL_ID,
                    &posted_message_ids,
                )
                .await;
                return Err(err);
            }
            let delete_outcome = self
                .delete_support_messages(&stored_message_ids, bot_user_id)
                .await?;
            output.deleted_message_ids = delete_outcome.deleted;
            output.warnings.extend(delete_outcome.warnings);
        } else if !adopted_from_history && support_publish::support_payload_is_unchanged(&output) {
            for message in &mut output.messages {
                if let Some(message_id) = message.stored_message_id {
                    message.action = "no_op".to_string();
                    message.message_id = Some(message_id);
                }
            }
            self.store_static_v2_metadata(
                support_publish::SUPPORT_PAYLOAD_FORMAT_KEY,
                support_publish::SUPPORT_PAYLOAD_FORMAT,
                support_publish::SUPPORT_PAYLOAD_HASH_KEY,
                &output.payload_hash,
            )
            .await?;
        } else {
            let mut message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let stored_message_id = output.messages[message_index].stored_message_id;
                if let Some(stored_message_id) = stored_message_id {
                    if let Some(message_id) = self
                        .edit_support_message(stored_message_id, &output.messages[message_index])
                        .await?
                    {
                        message_ids.push(message_id);
                        output.messages[message_index].action = if adopted_from_history {
                            "adopted_edit".to_string()
                        } else {
                            "edited".to_string()
                        };
                        output.messages[message_index].message_id = Some(message_id);
                        output.messages[message_index].stored_message_id = Some(message_id);
                        output.edited_message_ids.push(message_id);
                        continue;
                    }
                    output.warnings.push(format!(
                        "Support: gespeicherte Message-ID {stored_message_id} ist stale (404); es wird neu gepostet."
                    ));
                }

                let message_id = self
                    .post_support_message(&output.messages[message_index])
                    .await?;
                message_ids.push(message_id);
                self.store_static_v2_message_id(
                    support_publish::SUPPORT_MESSAGE_ID_PREFIX,
                    message_index,
                    message_id,
                )
                .await?;
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if adopted_from_history {
                self.replace_static_v2_message_ids_and_metadata(
                    support_publish::SUPPORT_MESSAGE_ID_PREFIX,
                    support_publish::SUPPORT_PAYLOAD_FORMAT_KEY,
                    support_publish::SUPPORT_PAYLOAD_FORMAT,
                    support_publish::SUPPORT_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                    &message_ids,
                )
                .await?;
            } else {
                self.store_static_v2_metadata(
                    support_publish::SUPPORT_PAYLOAD_FORMAT_KEY,
                    support_publish::SUPPORT_PAYLOAD_FORMAT,
                    support_publish::SUPPORT_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                )
                .await?;
            }
        }

        output.dry_run = false;
        output.repost_required = false;
        Ok(output)
    }

    async fn faq_apply(&self, confirm: bool) -> ServerSyncResult<FaqPublishOutput> {
        let stored_message_ids = self
            .load_static_v2_message_ids(faq_publish::FAQ_MESSAGE_ID_PREFIX)
            .await?;
        let stored_payload_format = self
            .load_serversync_kv(faq_publish::FAQ_PAYLOAD_FORMAT_KEY)
            .await?;
        let stored_payload_hash = self
            .load_serversync_kv(faq_publish::FAQ_PAYLOAD_HASH_KEY)
            .await?;
        let mut output = faq_publish::build_faq_publish_output(
            &self.rang_guide_repo_root,
            &stored_message_ids,
            stored_payload_format.as_deref(),
            stored_payload_hash.as_deref(),
            !confirm,
        )
        .map_err(ServerSyncError::bad_request)?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;

        let discovered_v2_message_ids = self.fetch_faq_v2_message_ids(bot_user_id).await?;
        let expected_message_count = output.messages.len();
        let mut adopted_from_history = false;
        if stored_message_ids.len() != expected_message_count {
            match discovered_v2_message_ids.len() {
                0 => {}
                count if count == expected_message_count => {
                    adopt_faq_message_ids(&mut output, &discovered_v2_message_ids, confirm);
                    adopted_from_history = true;
                }
                count => {
                    return Err(ServerSyncError::bad_request(format!(
                        "FAQ: KV-Message-IDs fehlen/unvollstaendig, aber History-Scan fand {count} eigene V2-Messages fuer {expected_message_count} erwartete Messages; Apply blockiert gegen Duplikate."
                    )));
                }
            }
        }

        if !confirm {
            return Ok(output);
        }
        for warning in &output.warnings {
            tracing::warn!(%warning, "FAQ-Publish-Warnung");
        }

        if output.repost_required {
            let mut posted_message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let post_result = self.post_faq_message(&output.messages[message_index]).await;
                let message_id = match post_result {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        self.cleanup_new_static_v2_posts_best_effort(
                            "FAQ",
                            faq_publish::FAQ_CHANNEL_ID,
                            &posted_message_ids,
                        )
                        .await;
                        return Err(err);
                    }
                };
                posted_message_ids.push(message_id);
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if let Err(err) = self
                .replace_static_v2_message_ids_and_metadata(
                    faq_publish::FAQ_MESSAGE_ID_PREFIX,
                    faq_publish::FAQ_PAYLOAD_FORMAT_KEY,
                    faq_publish::FAQ_PAYLOAD_FORMAT,
                    faq_publish::FAQ_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                    &posted_message_ids,
                )
                .await
            {
                self.cleanup_new_static_v2_posts_best_effort(
                    "FAQ",
                    faq_publish::FAQ_CHANNEL_ID,
                    &posted_message_ids,
                )
                .await;
                return Err(err);
            }
        } else if !adopted_from_history && faq_publish::faq_payload_is_unchanged(&output) {
            for message in &mut output.messages {
                if let Some(message_id) = message.stored_message_id {
                    message.action = "no_op".to_string();
                    message.message_id = Some(message_id);
                }
            }
            self.store_static_v2_metadata(
                faq_publish::FAQ_PAYLOAD_FORMAT_KEY,
                faq_publish::FAQ_PAYLOAD_FORMAT,
                faq_publish::FAQ_PAYLOAD_HASH_KEY,
                &output.payload_hash,
            )
            .await?;
        } else {
            let mut message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let stored_message_id = output.messages[message_index].stored_message_id;
                if let Some(stored_message_id) = stored_message_id {
                    if let Some(message_id) = self
                        .edit_faq_message(stored_message_id, &output.messages[message_index])
                        .await?
                    {
                        message_ids.push(message_id);
                        output.messages[message_index].action = if adopted_from_history {
                            "adopted_edit".to_string()
                        } else {
                            "edited".to_string()
                        };
                        output.messages[message_index].message_id = Some(message_id);
                        output.messages[message_index].stored_message_id = Some(message_id);
                        output.edited_message_ids.push(message_id);
                        continue;
                    }
                    output.warnings.push(format!(
                        "FAQ: gespeicherte Message-ID {stored_message_id} ist stale (404); es wird neu gepostet."
                    ));
                }

                let message_id = self
                    .post_faq_message(&output.messages[message_index])
                    .await?;
                message_ids.push(message_id);
                self.store_static_v2_message_id(
                    faq_publish::FAQ_MESSAGE_ID_PREFIX,
                    message_index,
                    message_id,
                )
                .await?;
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if adopted_from_history {
                self.replace_static_v2_message_ids_and_metadata(
                    faq_publish::FAQ_MESSAGE_ID_PREFIX,
                    faq_publish::FAQ_PAYLOAD_FORMAT_KEY,
                    faq_publish::FAQ_PAYLOAD_FORMAT,
                    faq_publish::FAQ_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                    &message_ids,
                )
                .await?;
            } else {
                self.store_static_v2_metadata(
                    faq_publish::FAQ_PAYLOAD_FORMAT_KEY,
                    faq_publish::FAQ_PAYLOAD_FORMAT,
                    faq_publish::FAQ_PAYLOAD_HASH_KEY,
                    &output.payload_hash,
                )
                .await?;
            }
        }

        output.dry_run = false;
        output.repost_required = false;
        Ok(output)
    }

    async fn welcome_preview(&self) -> ServerSyncResult<WelcomePublishOutput> {
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let team_members = self.fetch_welcome_team_members().await?;
        let stored_message_ids = self.load_welcome_message_ids().await?;
        let stored_payload_format = self.load_welcome_payload_format().await?;
        let bot_user_id = self.fetch_current_bot_user_id().await.ok();
        welcome_publish::build_welcome_publish_output(
            &live,
            &team_members,
            &welcome_publish::welcome_repo_root(),
            &stored_message_ids,
            stored_payload_format.as_deref(),
            true,
            bot_user_id,
        )
        .map_err(ServerSyncError::bad_request)
    }

    async fn welcome_apply(&self, confirm: bool) -> ServerSyncResult<WelcomePublishOutput> {
        let live = dl_server_as_code::import::fetch_live_guild_model(
            self.adapter.http.as_ref(),
            self.guild_id,
        )
        .await?;
        let team_members = self.fetch_welcome_team_members().await?;
        let stored_message_ids = self.load_welcome_message_ids().await?;
        let stored_payload_format = self.load_welcome_payload_format().await?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;
        let mut output = welcome_publish::build_welcome_publish_output(
            &live,
            &team_members,
            &welcome_publish::welcome_repo_root(),
            &stored_message_ids,
            stored_payload_format.as_deref(),
            !confirm,
            Some(bot_user_id),
        )
        .map_err(ServerSyncError::bad_request)?;

        let discovered_message_ids = self
            .fetch_welcome_marker_message_ids(output.channel_id, bot_user_id)
            .await?;
        let mut hero_message_id = output
            .sections
            .iter()
            .find(|section| section.section_id == "hero")
            .and_then(|section| section.message_id)
            .or_else(|| {
                discovered_message_ids
                    .get("hero")
                    .and_then(|ids| ids.first())
                    .copied()
            });

        if !confirm {
            if !output.repost_required {
                for section in &mut output.sections {
                    if section.stored_message_id.is_none() {
                        if let Some(discovered) = discovered_message_ids
                            .get(&section.section_id)
                            .and_then(|ids| ids.get(section.message_index))
                        {
                            section.action = "planned_edit".to_string();
                            section.message_id = Some(*discovered);
                        }
                    }
                }
                hero_message_id = output
                    .sections
                    .iter()
                    .find(|section| section.section_id == "hero")
                    .and_then(|section| section.message_id);
            } else {
                for section in &mut output.sections {
                    let discovered = discovered_message_ids
                        .get(&section.section_id)
                        .and_then(|ids| ids.get(section.message_index))
                        .copied();
                    section.message_id = section.stored_message_id.or(discovered);
                    section.action = if section.message_id.is_some() {
                        "planned_repost".to_string()
                    } else {
                        "planned_post".to_string()
                    };
                }
            }
            welcome_publish::refresh_quickstart_jump_button(&mut output, hero_message_id);
            return Ok(output);
        }

        if output.repost_required {
            self.delete_welcome_messages(
                output.channel_id,
                &stored_message_ids,
                &discovered_message_ids,
            )
            .await?;
            self.clear_welcome_message_id_keys(&stored_message_ids)
                .await?;
            hero_message_id = None;

            for section_index in 0..output.sections.len() {
                if output.sections[section_index].section_id == "quickstart" {
                    welcome_publish::refresh_quickstart_jump_button(&mut output, hero_message_id);
                }
                let section = &mut output.sections[section_index];
                let section_id = section.section_id.clone();
                let message_index = section.message_index;
                let message_id = self
                    .post_welcome_message(output.channel_id, section)
                    .await?;
                self.store_welcome_message_id(&section_id, message_index, message_id)
                    .await?;
                section.action = "posted".to_string();
                section.message_id = Some(message_id);
                output
                    .posted_message_ids
                    .entry(section_id.clone())
                    .or_default()
                    .push(message_id);
                if section_id == "hero" {
                    hero_message_id = Some(message_id);
                }
            }
            self.store_welcome_payload_format().await?;
        } else {
            for section_index in 0..output.sections.len() {
                if output.sections[section_index].section_id == "quickstart" {
                    welcome_publish::refresh_quickstart_jump_button(&mut output, hero_message_id);
                }
                let section = &mut output.sections[section_index];
                let section_id = section.section_id.clone();
                let message_index = section.message_index;
                let discovered_message_id = discovered_message_ids
                    .get(&section_id)
                    .and_then(|ids| ids.get(message_index))
                    .copied();
                let candidates = welcome_publish::welcome_candidate_message_ids(
                    section.stored_message_id,
                    discovered_message_id,
                );
                let mut edited_message_id = None;
                for candidate in candidates {
                    if let Some(message_id) = self
                        .edit_welcome_message(output.channel_id, candidate, section)
                        .await?
                    {
                        edited_message_id = Some(message_id);
                        break;
                    }
                }

                if let Some(message_id) = edited_message_id {
                    self.store_welcome_message_id(&section.section_id, message_index, message_id)
                        .await?;
                    section.action = "edited".to_string();
                    section.message_id = Some(message_id);
                    output
                        .edited_message_ids
                        .entry(section_id.clone())
                        .or_default()
                        .push(message_id);
                    if section_id == "hero" {
                        hero_message_id = Some(message_id);
                    }
                } else {
                    let message_id = self
                        .post_welcome_message(output.channel_id, section)
                        .await?;
                    self.store_welcome_message_id(&section_id, message_index, message_id)
                        .await?;
                    section.action = "posted".to_string();
                    section.message_id = Some(message_id);
                    output
                        .posted_message_ids
                        .entry(section_id.clone())
                        .or_default()
                        .push(message_id);
                    if section_id == "hero" {
                        hero_message_id = Some(message_id);
                    }
                }
            }
            self.store_welcome_payload_format().await?;
        }

        output.dry_run = false;
        output.repost_required = false;
        Ok(output)
    }

    async fn rang_guide_apply(&self, confirm: bool) -> ServerSyncResult<RangGuidePublishOutput> {
        let stored_message_ids = self.load_rang_guide_message_ids().await?;
        let stored_payload_format = self.load_rang_guide_payload_format().await?;
        let stored_payload_hash = self.load_rang_guide_payload_hash().await?;
        let mut output = rang_guide_publish::build_rang_guide_publish_output(
            &self.rang_guide_repo_root,
            &stored_message_ids,
            stored_payload_format.as_deref(),
            stored_payload_hash.as_deref(),
            !confirm,
        )
        .map_err(ServerSyncError::bad_request)?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;
        output.legacy_cleanup_candidates = self
            .fetch_rang_guide_legacy_cleanup_candidates(bot_user_id)
            .await?;
        let discovered_v2_message_ids = self.fetch_rang_guide_v2_message_ids(bot_user_id).await?;
        let expected_message_count = output.messages.len();
        let mut adopted_from_history = false;
        if stored_message_ids.len() != expected_message_count {
            match discovered_v2_message_ids.len() {
                0 => {}
                count if count == expected_message_count => {
                    Self::adopt_rang_guide_message_ids(
                        &mut output,
                        &discovered_v2_message_ids,
                        confirm,
                    );
                    adopted_from_history = true;
                }
                count => {
                    return Err(ServerSyncError::bad_request(format!(
                        "Rang-Guide: KV-Message-IDs fehlen/unvollstaendig, aber History-Scan fand {count} eigene V2-Guide-Messages fuer {expected_message_count} erwartete Messages; Apply blockiert gegen Duplikate."
                    )));
                }
            }
        }

        if !confirm {
            return Ok(output);
        }
        for warning in &output.warnings {
            tracing::warn!(%warning, "Rang-Guide-Publish-Warnung");
        }

        if output.repost_required {
            let mut posted_message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let post_result = self
                    .post_rang_guide_message(&output.messages[message_index])
                    .await;
                let message_id = match post_result {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        self.cleanup_new_rang_guide_posts_best_effort(&posted_message_ids)
                            .await;
                        return Err(err);
                    }
                };
                posted_message_ids.push(message_id);
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if let Err(err) = self
                .replace_rang_guide_message_ids_and_metadata(&output, &posted_message_ids)
                .await
            {
                self.cleanup_new_rang_guide_posts_best_effort(&posted_message_ids)
                    .await;
                return Err(err);
            }
            let delete_outcome = self
                .delete_rang_guide_messages(&stored_message_ids, bot_user_id)
                .await?;
            output.deleted_message_ids = delete_outcome.deleted;
            output.warnings.extend(delete_outcome.warnings);
        } else if !adopted_from_history
            && rang_guide_publish::rang_guide_payload_is_unchanged(&output)
        {
            for message in &mut output.messages {
                if let Some(message_id) = message.stored_message_id {
                    message.action = "no_op".to_string();
                    message.message_id = Some(message_id);
                }
            }
            self.store_rang_guide_metadata(&output).await?;
        } else {
            let mut message_ids = Vec::new();
            for message_index in 0..output.messages.len() {
                let stored_message_id = output.messages[message_index].stored_message_id;
                if let Some(stored_message_id) = stored_message_id {
                    if let Some(message_id) = self
                        .edit_rang_guide_message(stored_message_id, &output.messages[message_index])
                        .await?
                    {
                        message_ids.push(message_id);
                        output.messages[message_index].action = if adopted_from_history {
                            "adopted_edit".to_string()
                        } else {
                            "edited".to_string()
                        };
                        output.messages[message_index].message_id = Some(message_id);
                        output.messages[message_index].stored_message_id = Some(message_id);
                        output.edited_message_ids.push(message_id);
                        continue;
                    }
                    output.warnings.push(format!(
                        "Rang-Guide: gespeicherte Message-ID {stored_message_id} ist stale (404); es wird neu gepostet."
                    ));
                }

                let message_id = self
                    .post_rang_guide_message(&output.messages[message_index])
                    .await?;
                message_ids.push(message_id);
                self.store_rang_guide_message_id(message_index, message_id)
                    .await?;
                output.messages[message_index].action = "posted".to_string();
                output.messages[message_index].message_id = Some(message_id);
                output.messages[message_index].stored_message_id = Some(message_id);
                output.posted_message_ids.push(message_id);
            }
            if adopted_from_history {
                self.replace_rang_guide_message_ids_and_metadata(&output, &message_ids)
                    .await?;
            } else {
                self.store_rang_guide_metadata(&output).await?;
            }
        }

        self.cleanup_rang_guide_legacy_messages(&mut output).await?;
        output.dry_run = false;
        output.repost_required = false;
        Ok(output)
    }

    async fn load_voice_ux_message_ids(&self) -> ServerSyncResult<BTreeMap<u64, Vec<u64>>> {
        let mut out = BTreeMap::new();
        for channel_id in voice_ux_publish::VOICE_UX_TARGET_CHANNEL_IDS {
            let ids = self
                .load_static_v2_message_ids(&voice_ux_publish::voice_ux_message_id_prefix(
                    channel_id,
                ))
                .await?;
            if !ids.is_empty() {
                out.insert(channel_id, ids);
            }
        }
        Ok(out)
    }

    async fn load_voice_ux_payload_formats(
        &self,
    ) -> ServerSyncResult<BTreeMap<u64, Option<String>>> {
        let mut out = BTreeMap::new();
        for channel_id in voice_ux_publish::VOICE_UX_TARGET_CHANNEL_IDS {
            out.insert(
                channel_id,
                self.load_serversync_kv(&voice_ux_publish::voice_ux_payload_format_key(channel_id))
                    .await?,
            );
        }
        Ok(out)
    }

    async fn load_voice_ux_payload_hashes(
        &self,
    ) -> ServerSyncResult<BTreeMap<u64, Option<String>>> {
        let mut out = BTreeMap::new();
        for channel_id in voice_ux_publish::VOICE_UX_TARGET_CHANNEL_IDS {
            out.insert(
                channel_id,
                self.load_serversync_kv(&voice_ux_publish::voice_ux_payload_hash_key(channel_id))
                    .await?,
            );
        }
        Ok(out)
    }

    async fn load_voice_ux_lfg_forum_thread_id(&self) -> ServerSyncResult<Option<u64>> {
        self.load_serversync_kv(voice_ux_publish::VOICE_UX_LFG_FORUM_THREAD_ID_KEY)
            .await?
            .map(|value| parse_discord_id("voice_ux_lfg_forum_thread_id", &value))
            .transpose()
    }

    async fn fetch_voice_ux_v2_message_ids(
        &self,
        channel_id: u64,
        bot_user_id: u64,
    ) -> ServerSyncResult<Vec<u64>> {
        let mut before: Option<u64> = None;
        let mut message_ids = Vec::new();
        for _ in 0..5 {
            let mut url =
                self.discord_api_url(&format!("/channels/{channel_id}/messages?limit=100"));
            if let Some(before) = before {
                url.push_str("&before=");
                url.push_str(&before.to_string());
            }
            let page: Vec<DiscordMessage> = self.discord_get_json(url).await?;
            if page.is_empty() {
                break;
            }
            for message in &page {
                let message_id = parse_discord_id("Message-ID", &message.id)?;
                let author_id = parse_discord_id("Message-Author-ID", &message.author.id)?;
                let candidate = voice_ux_publish::VoiceUxV2Message {
                    message_id,
                    author_id,
                    flags: message.flags,
                    components: message.components.clone(),
                };
                if voice_ux_publish::is_voice_ux_v2_message(&candidate, bot_user_id) {
                    message_ids.push(message_id);
                }
            }
            before = page
                .last()
                .map(|message| parse_discord_id("Message-ID", &message.id))
                .transpose()?;
            if page.len() < 100 {
                break;
            }
        }
        message_ids.sort_unstable();
        message_ids.dedup();
        Ok(message_ids)
    }

    async fn edit_voice_ux_message(
        &self,
        channel_id: u64,
        message_id: u64,
        message: &voice_ux_publish::VoiceUxMessageOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = self.discord_api_url(&format!("/channels/{channel_id}/messages/{message_id}"));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload(
                "Voice-UX",
                "PATCH",
                url,
                payload_value,
                attachments,
            )
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Message-ID", &written.id).map(Some)
    }

    async fn post_voice_ux_message(
        &self,
        channel_id: u64,
        message: &voice_ux_publish::VoiceUxMessageOutput,
    ) -> ServerSyncResult<u64> {
        let url = self.discord_api_url(&format!("/channels/{channel_id}/messages"));
        let payload_value = serde_json::to_value(&message.payload)?;
        let attachments = message
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.id,
                    attachment.filename.clone(),
                    attachment.relative_path.clone(),
                )
            })
            .collect();
        let response = self
            .send_components_v2_message_payload("Voice-UX", "POST", url, payload_value, attachments)
            .await?;
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Message-ID", &written.id)
    }

    async fn replace_voice_ux_target_metadata(
        &self,
        target: &voice_ux_publish::VoiceUxTargetOutput,
        message_ids: &[u64],
    ) -> ServerSyncResult<()> {
        self.replace_static_v2_message_ids_and_metadata(
            &voice_ux_publish::voice_ux_message_id_prefix(target.channel_id),
            &voice_ux_publish::voice_ux_payload_format_key(target.channel_id),
            voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT,
            &voice_ux_publish::voice_ux_payload_hash_key(target.channel_id),
            &target.payload_hash,
            message_ids,
        )
        .await
    }

    async fn store_voice_ux_target_metadata(
        &self,
        target: &voice_ux_publish::VoiceUxTargetOutput,
    ) -> ServerSyncResult<()> {
        self.store_static_v2_metadata(
            &voice_ux_publish::voice_ux_payload_format_key(target.channel_id),
            voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT,
            &voice_ux_publish::voice_ux_payload_hash_key(target.channel_id),
            &target.payload_hash,
        )
        .await
    }

    async fn delete_voice_ux_messages(
        &self,
        channel_id: u64,
        stored_message_ids: &[u64],
        bot_user_id: u64,
    ) -> ServerSyncResult<StaticV2DeleteOutcome> {
        let mut message_ids = stored_message_ids.to_vec();
        message_ids.sort_unstable();
        message_ids.dedup();
        let mut deleted = Vec::new();
        let mut warnings = Vec::new();
        for message_id in message_ids {
            let response = match self
                .discord_get_response(
                    self.discord_api_url(&format!("/channels/{channel_id}/messages/{message_id}")),
                )
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    warnings.push(format!(
                        "Voice-UX: gespeicherte Message-ID {message_id} konnte vor Delete nicht gelesen werden: {err}"
                    ));
                    continue;
                }
            };
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                warnings.push(format!(
                    "Voice-UX: gespeicherte Message-ID {message_id} war beim Delete bereits weg"
                ));
                continue;
            }
            let message: DiscordMessage = match discord_regelwerk_json_response(response, "GET") {
                Ok(message) => message,
                Err(err) => {
                    warnings.push(format!(
                        "Voice-UX: gespeicherte Message-ID {message_id} konnte vor Delete nicht validiert werden: {err}"
                    ));
                    continue;
                }
            };
            let author_id = match parse_discord_id("Message-Author-ID", &message.author.id) {
                Ok(author_id) => author_id,
                Err(err) => {
                    warnings.push(format!(
                        "Voice-UX: gespeicherte Message-ID {message_id} hat ungueltigen Autor und wird nicht geloescht: {err}"
                    ));
                    continue;
                }
            };
            let candidate = voice_ux_publish::VoiceUxV2Message {
                message_id,
                author_id,
                flags: message.flags,
                components: message.components,
            };
            if !voice_ux_publish::is_voice_ux_v2_message(&candidate, bot_user_id) {
                warnings.push(format!(
                    "Voice-UX: gespeicherte Message-ID {message_id} passt nicht zur eigenen V2-Signatur und wird nicht geloescht"
                ));
                continue;
            }
            if self
                .discord_delete(
                    self.discord_api_url(&format!("/channels/{channel_id}/messages/{message_id}")),
                )
                .await?
            {
                deleted.push(message_id);
            }
        }
        Ok(StaticV2DeleteOutcome { deleted, warnings })
    }

    async fn cleanup_new_voice_ux_posts_best_effort(&self, channel_id: u64, message_ids: &[u64]) {
        self.cleanup_new_static_v2_posts_best_effort("Voice-UX", channel_id, message_ids)
            .await;
    }

    async fn pin_voice_ux_message_best_effort(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), String> {
        let message_url =
            self.discord_api_url(&format!("/channels/{channel_id}/messages/{message_id}"));
        let response = self
            .discord_get_response(message_url)
            .await
            .map_err(|err| err.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("Message wurde beim Pin-Check nicht gefunden".to_string());
        }
        let message: DiscordMessage =
            discord_regelwerk_json_response(response, "GET").map_err(|err| err.to_string())?;
        if message.pinned {
            return Ok(());
        }
        let url = self.discord_api_url(&format!("/channels/{channel_id}/pins/{message_id}"));
        let response = discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .put(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "PUT",
        )
        .await
        .map_err(|err| err.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "HTTP {}: {}",
                response.status().as_u16(),
                response.body_preview()
            ))
        }
    }

    async fn fetch_voice_ux_legacy_cleanup_candidates(
        &self,
        bot_user_id: u64,
    ) -> ServerSyncResult<(
        Vec<voice_ux_publish::VoiceUxLegacyCleanupCandidate>,
        Vec<String>,
    )> {
        let mut raw_candidates = vec![
            (
                voice_ux_publish::VOICE_UX_CHANNEL_ID,
                voice_ux_publish::VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID,
                "fixed_router_panel".to_string(),
            ),
            (
                voice_ux_publish::VOICE_UX_CHANNEL_ID,
                voice_ux_publish::VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID,
                "fixed_lfg_panel".to_string(),
            ),
        ];
        if let Ok(Some(raw)) = dl_central_db::kv::get(
            &self.pool,
            dl_voice::router::ROUTER_PANEL_KV_NS,
            dl_voice::router::ROUTER_PANEL_MESSAGE_KEY,
        )
        .await
        {
            if let Ok(message_id) =
                parse_discord_id("tempvoice_router.components_v2_message_id", &raw)
            {
                raw_candidates.push((
                    voice_ux_publish::VOICE_UX_CHANNEL_ID,
                    message_id,
                    "kv_tempvoice_router_components_v2_message_id".to_string(),
                ));
            }
        }
        if let Ok(Some(raw)) = dl_central_db::kv::get(
            &self.pool,
            dl_voice::lfg_panel::LFG_PANEL_KV_NS,
            dl_voice::lfg_panel::LFG_PANEL_MESSAGE_KEY,
        )
        .await
        {
            if let Ok(message_id) = parse_discord_id("lfg_panel.components_v2_message_id", &raw) {
                raw_candidates.push((
                    voice_ux_publish::VOICE_UX_CHANNEL_ID,
                    message_id,
                    "kv_lfg_panel_components_v2_message_id".to_string(),
                ));
            }
        }

        raw_candidates.sort_unstable_by_key(|(_, message_id, _)| *message_id);
        raw_candidates.dedup_by_key(|(_, message_id, _)| *message_id);

        let mut candidates = Vec::new();
        let mut warnings = Vec::new();
        for (channel_id, message_id, source) in raw_candidates {
            let response = match self
                .discord_get_response(
                    self.discord_api_url(&format!("/channels/{channel_id}/messages/{message_id}")),
                )
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    warnings.push(format!(
                        "Voice-UX: Legacy-Message {message_id} aus {source} konnte nicht gelesen werden: {err}"
                    ));
                    continue;
                }
            };
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                warnings.push(format!(
                    "Voice-UX: Legacy-Message {message_id} aus {source} wurde nicht gefunden"
                ));
                continue;
            }
            let message: DiscordMessage = match discord_regelwerk_json_response(response, "GET") {
                Ok(message) => message,
                Err(err) => {
                    warnings.push(format!(
                        "Voice-UX: Legacy-Message {message_id} aus {source} konnte nicht validiert werden: {err}"
                    ));
                    continue;
                }
            };
            let author_id = match parse_discord_id("Message-Author-ID", &message.author.id) {
                Ok(author_id) => author_id,
                Err(err) => {
                    warnings.push(format!(
                        "Voice-UX: Legacy-Message {message_id} aus {source} hat ungueltigen Autor: {err}"
                    ));
                    continue;
                }
            };
            let legacy = voice_ux_publish::VoiceUxLegacyMessage {
                channel_id,
                message_id,
                author_id,
                custom_ids: voice_ux_publish::collect_component_custom_ids(&message.components),
            };
            if voice_ux_publish::is_voice_ux_legacy_cleanup_candidate(&legacy, bot_user_id) {
                candidates.push(voice_ux_publish::VoiceUxLegacyCleanupCandidate {
                    channel_id,
                    message_id,
                    source,
                });
            } else {
                warnings.push(format!(
                    "Voice-UX: Legacy-Message {message_id} aus {source} passt nicht zur Bot-/Custom-ID-Signatur und wird nicht geloescht"
                ));
            }
        }
        Ok((candidates, warnings))
    }

    async fn cleanup_voice_ux_legacy_messages(
        &self,
        output: &mut voice_ux_publish::VoiceUxPublishOutput,
    ) -> ServerSyncResult<()> {
        for candidate in output.legacy_cleanup_candidates.clone() {
            if self
                .discord_delete(self.discord_api_url(&format!(
                    "/channels/{}/messages/{}",
                    candidate.channel_id, candidate.message_id
                )))
                .await?
            {
                output.deleted_legacy_message_ids.push(candidate.message_id);
            } else {
                output.warnings.push(format!(
                    "Voice-UX: Legacy-Message {} war beim Cleanup bereits weg",
                    candidate.message_id
                ));
            }
        }
        for (ns, key) in [
            (
                dl_voice::router::ROUTER_PANEL_KV_NS,
                dl_voice::router::ROUTER_PANEL_MESSAGE_KEY,
            ),
            (
                dl_voice::lfg_panel::LFG_PANEL_KV_NS,
                dl_voice::lfg_panel::LFG_PANEL_MESSAGE_KEY,
            ),
        ] {
            match dl_central_db::kv::delete(&self.pool, ns, key).await {
                Ok(()) => output.cleared_legacy_kv_keys.push(format!("{ns}:{key}")),
                Err(err) => output.warnings.push(format!(
                    "Voice-UX: Legacy-KV {ns}:{key} konnte nicht entfernt werden: {err}"
                )),
            }
        }
        Ok(())
    }

    async fn post_voice_ux_forum_post(
        &self,
        forum: &voice_ux_publish::VoiceUxForumPostOutput,
    ) -> ServerSyncResult<u64> {
        let url = self.discord_api_url(&format!(
            "/channels/{}/threads",
            voice_ux_publish::VOICE_UX_LFG_FORUM_CHANNEL_ID
        ));
        let body = json!({
            "name": forum.title,
            "auto_archive_duration": 1440,
            "applied_tags": [voice_ux_publish::VOICE_UX_LFG_FORUM_INFO_TAG_ID],
            "message": forum.payload,
        });
        let response = discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .post(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("Content-Type", "application/json")
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                    .json(&body);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "POST",
        )
        .await?;
        let thread: DiscordThread = discord_regelwerk_json_response(response, "POST")?;
        parse_discord_id("Forum-Thread-ID", &thread.id)
    }

    async fn edit_voice_ux_forum_post(
        &self,
        thread_id: u64,
        forum: &voice_ux_publish::VoiceUxForumPostOutput,
    ) -> ServerSyncResult<Option<u64>> {
        let url = self.discord_api_url(&format!("/channels/{thread_id}/messages/{thread_id}"));
        let payload_value = serde_json::to_value(&forum.payload)?;
        let response = self
            .send_components_v2_message_payload(
                "Voice-UX-Forum",
                "PATCH",
                url,
                payload_value,
                Vec::new(),
            )
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let written: DiscordMessageWriteResponse =
            discord_regelwerk_json_response(response, "PATCH")?;
        parse_discord_id("Forum-Starter-Message-ID", &written.id).map(Some)
    }

    async fn store_voice_ux_forum_metadata(
        &self,
        thread_id: u64,
        payload_hash: &str,
    ) -> ServerSyncResult<()> {
        self.store_serversync_kv(
            voice_ux_publish::VOICE_UX_LFG_FORUM_THREAD_ID_KEY,
            &thread_id.to_string(),
        )
        .await?;
        self.store_serversync_kv(
            voice_ux_publish::VOICE_UX_LFG_FORUM_PAYLOAD_FORMAT_KEY,
            voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT,
        )
        .await?;
        self.store_serversync_kv(
            voice_ux_publish::VOICE_UX_LFG_FORUM_PAYLOAD_HASH_KEY,
            payload_hash,
        )
        .await
    }

    async fn pin_voice_ux_forum_thread_best_effort(&self, thread_id: u64) -> Result<(), String> {
        let url = self.discord_api_url(&format!("/channels/{thread_id}"));
        let payload = json!({ "pinned": true });
        let response = discord_regelwerk_send_with_retry(
            || {
                let request = self
                    .http_client
                    .patch(url.clone())
                    .header("Authorization", format!("Bot {}", self.discord_token))
                    .header("Content-Type", "application/json")
                    .header("X-Audit-Log-Reason", ONBOARDING_AUDIT_LOG_REASON)
                    .json(&payload);
                async move {
                    let response = request.send().await?;
                    DiscordRestResponse::from_response(response).await
                }
            },
            "PATCH",
        )
        .await
        .map_err(|err| err.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "HTTP {}: {}",
                response.status().as_u16(),
                response.body_preview()
            ))
        }
    }

    async fn router_apply(&self, confirm: bool) -> ServerSyncResult<RouterApplyOutput> {
        let stored_message_ids = self.load_voice_ux_message_ids().await?;
        let stored_payload_formats = self.load_voice_ux_payload_formats().await?;
        let stored_payload_hashes = self.load_voice_ux_payload_hashes().await?;
        let forum_thread_id = self.load_voice_ux_lfg_forum_thread_id().await?;
        let forum_stored_payload_hash = self
            .load_serversync_kv(voice_ux_publish::VOICE_UX_LFG_FORUM_PAYLOAD_HASH_KEY)
            .await?;
        let mut output = voice_ux_publish::build_voice_ux_publish_output(
            &self.rang_guide_repo_root,
            &stored_message_ids,
            &stored_payload_formats,
            &stored_payload_hashes,
            forum_thread_id,
            forum_stored_payload_hash.as_deref(),
            !confirm,
        )
        .map_err(ServerSyncError::bad_request)?;
        let bot_user_id = self.fetch_current_bot_user_id().await?;

        for target_index in 0..output.targets.len() {
            let channel_id = output.targets[target_index].channel_id;
            let discovered = self
                .fetch_voice_ux_v2_message_ids(channel_id, bot_user_id)
                .await?;
            let expected = output.targets[target_index].messages.len();
            // Adoption nur bei komplett verlorenem KV; ein Format-/Layoutwechsel mit
            // intaktem KV (Länge ≠ erwartet) läuft über den Repost-Pfad, der die
            // gespeicherten alten Messages selbst abräumt.
            if output.targets[target_index].stored_message_ids.is_empty() {
                match discovered.len() {
                    0 => {}
                    count if count == expected => {
                        voice_ux_publish::adopt_voice_ux_message_ids(
                            &mut output.targets[target_index],
                            &discovered,
                            confirm,
                        );
                        output.warnings.push(format!(
                            "Voice-UX: KV-Message-IDs fuer Kanal {channel_id} fehlen/unvollstaendig; vorhandene eigene V2-Messages werden adoptiert: {discovered:?}"
                        ));
                    }
                    count => {
                        return Err(ServerSyncError::bad_request(format!(
                            "Voice-UX: KV-Message-IDs fuer Kanal {channel_id} fehlen/unvollstaendig, aber History-Scan fand {count} eigene V2-Messages fuer {expected} erwartete Messages; Apply blockiert gegen Duplikate."
                        )));
                    }
                }
            }
        }

        let (legacy_candidates, legacy_warnings) = self
            .fetch_voice_ux_legacy_cleanup_candidates(bot_user_id)
            .await?;
        output.legacy_cleanup_candidates = legacy_candidates;
        output.warnings.extend(legacy_warnings);

        if !confirm {
            return Ok(output);
        }

        for target_index in 0..output.targets.len() {
            if output.targets[target_index].repost_required {
                let channel_id = output.targets[target_index].channel_id;
                let old_ids = output.targets[target_index].stored_message_ids.clone();
                let mut posted_message_ids = Vec::new();
                for message_index in 0..output.targets[target_index].messages.len() {
                    let post_result = self
                        .post_voice_ux_message(
                            channel_id,
                            &output.targets[target_index].messages[message_index],
                        )
                        .await;
                    let message_id = match post_result {
                        Ok(message_id) => message_id,
                        Err(err) => {
                            self.cleanup_new_voice_ux_posts_best_effort(
                                channel_id,
                                &posted_message_ids,
                            )
                            .await;
                            return Err(err);
                        }
                    };
                    posted_message_ids.push(message_id);
                    let message = &mut output.targets[target_index].messages[message_index];
                    message.action = "posted".to_string();
                    message.message_id = Some(message_id);
                    message.stored_message_id = Some(message_id);
                    output.targets[target_index]
                        .posted_message_ids
                        .push(message_id);
                }
                if let Err(err) = self
                    .replace_voice_ux_target_metadata(
                        &output.targets[target_index],
                        &posted_message_ids,
                    )
                    .await
                {
                    self.cleanup_new_voice_ux_posts_best_effort(channel_id, &posted_message_ids)
                        .await;
                    return Err(err);
                }
                let delete_outcome = self
                    .delete_voice_ux_messages(channel_id, &old_ids, bot_user_id)
                    .await?;
                output.targets[target_index].deleted_message_ids = delete_outcome.deleted;
                output.warnings.extend(delete_outcome.warnings);
            } else if voice_ux_publish::voice_ux_target_payload_is_unchanged(
                &output.targets[target_index],
            ) {
                for message in &mut output.targets[target_index].messages {
                    if let Some(message_id) = message.stored_message_id {
                        message.action = "no_op".to_string();
                        message.message_id = Some(message_id);
                    }
                }
                self.store_voice_ux_target_metadata(&output.targets[target_index])
                    .await?;
            } else {
                let channel_id = output.targets[target_index].channel_id;
                let mut message_ids = Vec::new();
                for message_index in 0..output.targets[target_index].messages.len() {
                    let stored_message_id =
                        output.targets[target_index].messages[message_index].stored_message_id;
                    if let Some(stored_message_id) = stored_message_id {
                        if let Some(message_id) = self
                            .edit_voice_ux_message(
                                channel_id,
                                stored_message_id,
                                &output.targets[target_index].messages[message_index],
                            )
                            .await?
                        {
                            message_ids.push(message_id);
                            let message = &mut output.targets[target_index].messages[message_index];
                            message.action = "edited".to_string();
                            message.message_id = Some(message_id);
                            message.stored_message_id = Some(message_id);
                            output.targets[target_index]
                                .edited_message_ids
                                .push(message_id);
                            continue;
                        }
                        output.warnings.push(format!(
                            "Voice-UX: gespeicherte Message-ID {stored_message_id} in Kanal {channel_id} ist stale (404); es wird neu gepostet."
                        ));
                    }
                    let message_id = self
                        .post_voice_ux_message(
                            channel_id,
                            &output.targets[target_index].messages[message_index],
                        )
                        .await?;
                    message_ids.push(message_id);
                    self.store_static_v2_message_id(
                        &voice_ux_publish::voice_ux_message_id_prefix(channel_id),
                        message_index,
                        message_id,
                    )
                    .await?;
                    let message = &mut output.targets[target_index].messages[message_index];
                    message.action = "posted".to_string();
                    message.message_id = Some(message_id);
                    message.stored_message_id = Some(message_id);
                    output.targets[target_index]
                        .posted_message_ids
                        .push(message_id);
                }
                self.replace_voice_ux_target_metadata(&output.targets[target_index], &message_ids)
                    .await?;
            }

            let channel_id = output.targets[target_index].channel_id;
            let message_ids = output.targets[target_index]
                .messages
                .iter()
                .filter_map(|message| message.message_id)
                .collect::<Vec<_>>();
            for message_id in message_ids {
                match self.pin_voice_ux_message_best_effort(channel_id, message_id).await {
                    Ok(()) => {
                        if !output.targets[target_index]
                            .pinned_message_ids
                            .contains(&message_id)
                        {
                            output.targets[target_index]
                                .pinned_message_ids
                                .push(message_id);
                        }
                    }
                    Err(err) => output.warnings.push(format!(
                        "Voice-UX: Message {message_id} in Kanal {channel_id} konnte nicht gepinnt werden: {err}"
                    )),
                }
            }
            output.targets[target_index].repost_required = false;
        }

        if output.forum_post.stored_thread_id.is_some()
            && voice_ux_publish::voice_ux_forum_payload_is_unchanged(&output.forum_post)
        {
            if let Some(thread_id) = output.forum_post.stored_thread_id {
                output.forum_post.thread_id = Some(thread_id);
                output.forum_post.action = "no_op".to_string();
                self.store_voice_ux_forum_metadata(thread_id, &output.forum_post.payload_hash)
                    .await?;
                match self.pin_voice_ux_forum_thread_best_effort(thread_id).await {
                    Ok(()) => output.forum_post.pinned = true,
                    Err(err) => output.forum_post.warnings.push(format!(
                        "Forum-Thread {thread_id} konnte nicht gepinnt werden: {err}"
                    )),
                }
            }
        } else if let Some(thread_id) = output.forum_post.stored_thread_id {
            match self
                .edit_voice_ux_forum_post(thread_id, &output.forum_post)
                .await?
            {
                Some(_) => {
                    output.forum_post.thread_id = Some(thread_id);
                    output.forum_post.action = "edited".to_string();
                    self.store_voice_ux_forum_metadata(thread_id, &output.forum_post.payload_hash)
                        .await?;
                    match self.pin_voice_ux_forum_thread_best_effort(thread_id).await {
                        Ok(()) => output.forum_post.pinned = true,
                        Err(err) => output.forum_post.warnings.push(format!(
                            "Forum-Thread {thread_id} konnte nicht gepinnt werden: {err}"
                        )),
                    }
                }
                None => {
                    let new_thread_id = self.post_voice_ux_forum_post(&output.forum_post).await?;
                    output.forum_post.thread_id = Some(new_thread_id);
                    output.forum_post.action = "posted".to_string();
                    self.store_voice_ux_forum_metadata(
                        new_thread_id,
                        &output.forum_post.payload_hash,
                    )
                    .await?;
                    match self
                        .pin_voice_ux_forum_thread_best_effort(new_thread_id)
                        .await
                    {
                        Ok(()) => output.forum_post.pinned = true,
                        Err(err) => output.forum_post.warnings.push(format!(
                            "Forum-Thread {new_thread_id} konnte nicht gepinnt werden: {err}"
                        )),
                    }
                }
            }
        } else {
            let thread_id = self.post_voice_ux_forum_post(&output.forum_post).await?;
            output.forum_post.thread_id = Some(thread_id);
            output.forum_post.action = "posted".to_string();
            self.store_voice_ux_forum_metadata(thread_id, &output.forum_post.payload_hash)
                .await?;
            match self.pin_voice_ux_forum_thread_best_effort(thread_id).await {
                Ok(()) => output.forum_post.pinned = true,
                Err(err) => output.forum_post.warnings.push(format!(
                    "Forum-Thread {thread_id} konnte nicht gepinnt werden: {err}"
                )),
            }
        }

        output.warnings.extend(output.forum_post.warnings.clone());
        self.cleanup_voice_ux_legacy_messages(&mut output).await?;
        output.dry_run = false;
        Ok(output)
    }

    async fn lfg_panel_apply(&self, confirm: bool) -> ServerSyncResult<LfgPanelApplyOutput> {
        Ok(LfgPanelApplyOutput {
            guild_id: self.guild_id,
            channel_id: None,
            dry_run: !confirm,
            payload_format: dl_voice::lfg_panel::LFG_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: None,
            stored_message_id: None,
            action: "blocked_superseded_by_voice_ux".to_string(),
            message_id: None,
            blocked_reason: Some(
                "lfg_panel_apply_superseded_by_serversync_router_apply".to_string(),
            ),
            warnings: vec![
                "LFG-Panel: alter Einzelpanel-Publisher ist durch Voice-UX in /serversync/router-apply ersetzt.".to_string(),
            ],
            payload: json!({}),
        })
    }
}

fn snapshot_output(report: SnapshotImportReport) -> SnapshotOutput {
    SnapshotOutput {
        snapshot_id: report.snapshot_id,
        guild_id: report.guild_id,
        captured_at: report
            .captured_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        categories: report.categories,
        channels: report.channels,
        roles: report.roles,
        overwrites: report.overwrites,
    }
}

struct DiscordRestResponse {
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    body: Bytes,
}

impl DiscordRestResponse {
    async fn from_response(response: reqwest::Response) -> ServerSyncResult<Self> {
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.bytes().await?;
        Ok(Self {
            status,
            headers,
            body,
        })
    }

    #[cfg(test)]
    fn for_test(status: reqwest::StatusCode, body: String) -> Self {
        Self {
            status,
            headers: reqwest::header::HeaderMap::new(),
            body: Bytes::from(body),
        }
    }

    fn status(&self) -> reqwest::StatusCode {
        self.status
    }

    fn body_preview(&self) -> String {
        String::from_utf8_lossy(&self.body)
            .chars()
            .take(300)
            .collect()
    }
}

async fn discord_regelwerk_send_with_retry<S, Fut>(
    mut send: S,
    method: &str,
) -> ServerSyncResult<DiscordRestResponse>
where
    S: FnMut() -> Fut,
    Fut: Future<Output = ServerSyncResult<DiscordRestResponse>>,
{
    for attempt in 1..=REGELWERK_DISCORD_MAX_ATTEMPTS {
        let response = send().await?;
        if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }
        if attempt == REGELWERK_DISCORD_MAX_ATTEMPTS {
            return Ok(response);
        }
        let retry_after = regelwerk_retry_after(&response).unwrap_or(Duration::from_secs(1));
        tracing::warn!(
            method,
            attempt,
            retry_after_ms = retry_after.as_millis(),
            "Discord Regelwerk REST rate-limited"
        );
        tokio::time::sleep(retry_after).await;
    }
    unreachable!("retry loop always returns")
}

fn discord_regelwerk_json_response<T: serde::de::DeserializeOwned>(
    response: DiscordRestResponse,
    method: &str,
) -> ServerSyncResult<T> {
    let status = response.status();
    if !status.is_success() {
        return Err(ServerSyncError::internal(format!(
            "Discord {method} fehlgeschlagen: HTTP {}: {}",
            status.as_u16(),
            response.body_preview()
        )));
    }
    Ok(serde_json::from_slice::<T>(&response.body)?)
}

fn regelwerk_retry_after(response: &DiscordRestResponse) -> Option<Duration> {
    retry_after_from_body(&response.body)
        .or_else(|| retry_after_from_header(&response.headers))
        .and_then(retry_after_duration)
}

fn retry_after_duration(seconds: f64) -> Option<Duration> {
    seconds
        .is_finite()
        .then(|| Duration::from_secs_f64(seconds.max(0.0)))
}

fn retry_after_from_body(body: &[u8]) -> Option<f64> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    let retry_after = value.get("retry_after")?;
    retry_after
        .as_f64()
        .or_else(|| retry_after.as_str().and_then(|raw| raw.parse::<f64>().ok()))
}

fn retry_after_from_header(headers: &reqwest::header::HeaderMap) -> Option<f64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<f64>()
        .ok()
}

async fn server_guide_response_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    method: &str,
) -> ServerSyncResult<T> {
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::NOT_FOUND
    ) {
        return Err(ServerSyncError::bad_request(
            SERVER_GUIDE_UNAVAILABLE_MESSAGE,
        ));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body_preview: String = body.chars().take(300).collect();
        return Err(ServerSyncError::internal(format!(
            "Discord Server Guide {method} fehlgeschlagen: HTTP {}: {}",
            status.as_u16(),
            body_preview
        )));
    }
    Ok(response.json::<T>().await?)
}

fn parse_discord_id(label: &str, value: &str) -> ServerSyncResult<u64> {
    value
        .parse::<u64>()
        .map_err(|_| ServerSyncError::bad_request(format!("{label} ist keine Discord-ID")))
}

fn thread_ids(threads: Vec<DiscordThread>) -> ServerSyncResult<Vec<u64>> {
    threads
        .into_iter()
        .map(|thread| parse_discord_id("Thread-ID", &thread.id))
        .collect()
}

fn regelwerk_output(input: RegelwerkOutputInput<'_>) -> RegelwerkPublishOutput {
    RegelwerkPublishOutput {
        guild_id: GUILD_ID,
        dry_run: input.dry_run,
        threads_found: input.thread_count,
        threads_deleted: input.threads_deleted,
        bot_messages_found: input.bot_messages.len(),
        bot_messages_deleted: input.bot_messages_deleted,
        bot_message_ids: input
            .bot_messages
            .iter()
            .map(|message| message.message_id)
            .collect(),
        bot_message_embed_titles: input
            .bot_messages
            .iter()
            .flat_map(|message| message.embed_titles.iter().cloned())
            .collect(),
        stored_message_id: input.stored_message_id,
        posted_message_id: input.posted_message_id,
        edited_message_id: input.edited_message_id,
    }
}

fn apply_output(report: ApplyReport) -> ServerSyncResult<ApplyOutput> {
    let skipped = report
        .change_results
        .iter()
        .filter(|entry| entry.status == "dry_run" || entry.status.starts_with("skipped"))
        .count();
    let failed = report
        .change_results
        .iter()
        .filter(|entry| entry.status.starts_with("failed"))
        .count();
    let details = json!({
        "apply_run_id": report.apply_run_id,
        "preview_id": report.preview_id,
        "dry_run": report.dry_run,
        "applied": report.applied_changes,
        "skipped": skipped,
        "failed": failed,
        "change_results": report.change_results,
    });
    Ok(ApplyOutput {
        apply_run_id: report.apply_run_id,
        preview_id: report.preview_id,
        dry_run: report.dry_run,
        applied: report.applied_changes,
        skipped,
        failed,
        details_text: serde_json::to_string_pretty(&details)?,
        details,
    })
}

fn build_regelwerk_text(model: &GuildModel) -> ServerSyncResult<String> {
    let deadlock_rang = require_channel_mention(model, "deadlock-rang")?;
    let support_ticket = format!("<#{SUPPORT_TICKET_CHANNEL_ID}>");
    let server_bot_fragen = format!("<#{}>", faq_publish::FAQ_CHANNEL_ID);
    let frag_die_community = require_channel_mention(model, "frag-die-community")?;

    Ok(format!(
        "**📜 Regelwerk · Deutsche Deadlock Community**\n\n\
**Verhalten**\n\
- Respekt gegenüber allen — keine Beleidigungen, Diskriminierung oder persönlichen Angriffe\n\
- Keine Hassrede, kein NSFW außerhalb der dafür markierten Kanäle, kein Spam, keine Fremdwerbung\n\
- Privatsphäre respektieren — keine fremden Daten posten\n\
- Schädliche Inhalte (Viren, IP-Grabber, Scam-Links) = sofortiger permanenter Bann\n\n\
**Im Spielkontext erlaubt**\n\
Situatives Trash-Talking, Sarkasmus, Wortspiele — solange es nicht persönlich wird. Ohne nonverbale Signale kann Ton schnell schiefgehen, also vorher abchecken, ob alle damit fein sind.\n\n\
**Universalregel:** Sei kein Arschloch 😄\n\n\
**Schnell zurechtfinden**\n\
- {deadlock_rang} — Steam verknüpfen, Rang eintragen\n\
- {support_ticket} — wenn irgendwas nicht funktioniert (Ticket aufmachen)\n\
- {server_bot_fragen} — Fragen über Server, Bots und Concierge\n\
- {frag_die_community} — Community-Fragen und Deadlock-Invite\n\n\
**Moderation**\n\
Probleme? @Moderator oder @Owner pingen — oder ein Ticket aufmachen, wenn's diskreter sein soll. Konsequenzen je nach Schwere: Verwarnung → Timeout → Ban."
    ))
}

fn require_channel_mention(model: &GuildModel, name: &str) -> ServerSyncResult<String> {
    resolve_channel_id(model, name)
        .map_err(ServerSyncError::bad_request)?
        .map(|id| format!("<#{id}>"))
        .ok_or_else(|| ServerSyncError::bad_request(format!("Kanal `{name}` wurde nicht gefunden")))
}

fn build_server_guide_config(_live_config: &Value, model: &GuildModel) -> ServerGuideBuildOutput {
    let mut blockers = Vec::new();
    let warnings = Vec::new();

    let frag_die_community =
        require_serverguide_channel_id(model, "frag-die-community", &mut blockers);
    let deadlock_rang = require_serverguide_channel_id(model, "deadlock-rang", &mut blockers);
    let mitspieler_suche = require_serverguide_channel_id(model, "mitspieler-suche", &mut blockers);
    let patchnotes = require_serverguide_channel_id(model, "patchnotes", &mut blockers);
    let support_ticket = require_serverguide_channel_id_by_id(
        model,
        SUPPORT_TICKET_CHANNEL_ID,
        "Support-Tickets",
        &mut blockers,
    );
    let regelwerk =
        require_serverguide_channel_id_by_id(model, RULES_CHANNEL_ID, "Regelwerk", &mut blockers);

    let (
        Some(frag_die_community),
        Some(deadlock_rang),
        Some(mitspieler_suche),
        Some(patchnotes),
        Some(support_ticket),
        Some(regelwerk),
    ) = (
        frag_die_community,
        deadlock_rang,
        mitspieler_suche,
        patchnotes,
        support_ticket,
        regelwerk,
    )
    else {
        return ServerGuideBuildOutput {
            config: None,
            blockers,
            warnings,
        };
    };

    let config = ServerGuideConfig {
        enabled: true,
        welcome_message: ServerGuideWelcomeMessage {
            author_ids: vec!["662995601738170389".to_string()],
            message: "Schön, dass du da bist! Schau dich in Ruhe um — und wenn du Deadlock noch nicht hast, fragst du in #frag-die-community nach einem Invite.".to_string(),
        },
        new_member_actions: vec![
            ServerGuideAction {
                channel_id: frag_die_community.to_string(),
                action_type: SERVER_GUIDE_ACTION_TYPE_CHAT,
                title: "Sag Hallo".to_string(),
                description: Some(String::new()),
            },
            ServerGuideAction {
                channel_id: deadlock_rang.to_string(),
                // Rang-Wahl laeuft ueber das Panel, nicht per Chat — Kanal ist nicht @everyone-sendbar.
                action_type: SERVER_GUIDE_ACTION_TYPE_VIEW,
                title: "Steam verknüpfen & Rang eintragen".to_string(),
                description: Some(String::new()),
            },
            ServerGuideAction {
                channel_id: mitspieler_suche.to_string(),
                // LFG laeuft ueber das Forum-/Panel-Entry, nicht per freiem Chat.
                action_type: SERVER_GUIDE_ACTION_TYPE_VIEW,
                title: "Such dir Mitspieler".to_string(),
                description: Some(String::new()),
            },
        ],
        resource_channels: vec![
            ServerGuideResourceChannel {
                channel_id: regelwerk.to_string(),
                title: "Regelwerk".to_string(),
                description: Some(String::new()),
            },
            ServerGuideResourceChannel {
                channel_id: patchnotes.to_string(),
                title: "Patchnotes".to_string(),
                description: Some(String::new()),
            },
            ServerGuideResourceChannel {
                channel_id: support_ticket.to_string(),
                title: "Hilfe & Support".to_string(),
                description: Some(String::new()),
            },
        ],
    };

    if let Err(err) = validate_server_guide_config(model, &config) {
        blockers.push(err);
        return ServerGuideBuildOutput {
            config: None,
            blockers,
            warnings,
        };
    }

    ServerGuideBuildOutput {
        config: Some(config),
        blockers,
        warnings,
    }
}

fn require_serverguide_channel_id(
    model: &GuildModel,
    name: &str,
    blockers: &mut Vec<String>,
) -> Option<u64> {
    match resolve_channel_id(model, name) {
        Ok(Some(id)) => Some(id),
        Ok(None) => {
            blockers.push(format!(
                "Kanal `{name}` wurde im Live-Guild-Modell nicht gefunden"
            ));
            None
        }
        Err(err) => {
            blockers.push(err);
            None
        }
    }
}

fn require_serverguide_channel_id_by_id(
    model: &GuildModel,
    channel_id: u64,
    label: &str,
    blockers: &mut Vec<String>,
) -> Option<u64> {
    if model.channels.contains_key(&channel_id) {
        Some(channel_id)
    } else {
        blockers.push(format!(
            "Kanal `{label}` ({channel_id}) wurde im Live-Guild-Modell nicht gefunden"
        ));
        None
    }
}

fn validate_server_guide_config(
    model: &GuildModel,
    config: &ServerGuideConfig,
) -> Result<(), String> {
    if config.new_member_actions.len() < 3 {
        return Err(format!(
            "Server Guide braucht mindestens 3 new_member_actions, gefunden {}",
            config.new_member_actions.len()
        ));
    }

    let mut missing = Vec::new();
    for action in &config.new_member_actions {
        match action.channel_id.parse::<u64>() {
            Ok(channel_id) if model.channels.contains_key(&channel_id) => {
                if !everyone_can_view(model, channel_id) {
                    missing.push(format!(
                        "Action `{}`: Kanal `{channel_id}` ist nicht @everyone-sichtbar",
                        action.title
                    ));
                }
                if action.action_type == SERVER_GUIDE_ACTION_TYPE_CHAT
                    && !everyone_can_view_and_send(model, channel_id)
                {
                    missing.push(format!(
                        "Action `{}`: Kanal `{channel_id}` ist nicht @everyone-sendbar",
                        action.title
                    ));
                }
            }
            Ok(channel_id) => missing.push(format!(
                "Action `{}`: Kanal `{channel_id}` fehlt",
                action.title
            )),
            Err(_) => missing.push(format!(
                "Action `{}`: Kanal `{}` ist keine Discord-ID",
                action.title, action.channel_id
            )),
        }
    }
    for resource in &config.resource_channels {
        match resource.channel_id.parse::<u64>() {
            Ok(channel_id) if model.channels.contains_key(&channel_id) => {
                if !everyone_can_view(model, channel_id) {
                    missing.push(format!(
                        "Resource `{}`: Kanal `{channel_id}` ist nicht @everyone-sichtbar",
                        resource.title
                    ));
                }
            }
            Ok(channel_id) => missing.push(format!(
                "Resource `{}`: Kanal `{channel_id}` fehlt",
                resource.title
            )),
            Err(_) => missing.push(format!(
                "Resource `{}`: Kanal `{}` ist keine Discord-ID",
                resource.title, resource.channel_id
            )),
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Server Guide Preview blockiert:\n- {}",
            missing.join("\n- ")
        ))
    }
}

fn build_welle2b_onboarding_config(
    live_config: &Value,
    model: &GuildModel,
) -> ServerSyncResult<OnboardingBuildOutput> {
    let live = parse_live_onboarding_config(live_config)?;
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    // Die Default-Kanaele gestaltet der Owner im Discord-UI; der Bot uebernimmt
    // sie 1:1 und published nur die Prompts (Owner-Vorfall 2026-07-15: die alte
    // Code-Namensliste ueberschrieb die Owner-Auswahl beim Apply). Nur wenn live
    // noch gar keine Defaults existieren, greift die Namensliste als Fallback.
    let default_channel_ids = if live.default_channel_ids.is_empty() {
        let ids = resolve_default_channel_ids(model, &mut blockers);
        if !ids.is_empty() {
            if let Err(err) = validate_default_channels_7_5(model, &ids) {
                blockers.push(err);
            }
        }
        ids
    } else {
        let (kept, dropped): (Vec<String>, Vec<String>) =
            live.default_channel_ids.iter().cloned().partition(|id| {
                // Discord erlaubt auch Kategorien als Onboarding-Defaults.
                id.parse::<u64>().ok().is_some_and(|id| {
                    model.channels.contains_key(&id) || model.categories.contains_key(&id)
                })
            });
        if !dropped.is_empty() {
            warnings.push(format!(
                "Default-Kanaele: stale Live-IDs entfernt: {}",
                dropped.join(", ")
            ));
        }
        kept
    };

    let mitspieler_suche = match resolve_channel_id(model, "mitspieler-suche") {
        Ok(id) => id,
        Err(err) => {
            blockers.push(err);
            None
        }
    };
    let invite_role = require_role_id(model, &["Invite-Gast"], "Invite-Gast", &mut blockers);
    let frischling_role = require_role_id(model, &["Frischling"], "Frischling", &mut blockers);
    let rank_prompt = find_rank_prompt(&live, model);
    if rank_prompt.is_none() {
        blockers.push(
            "Rang-Prompt mit 12 Optionen auf `(unverifiziert)`-Rollen wurde in der Live-Onboarding-Config nicht gefunden"
                .to_string(),
        );
    }

    let mut prompts = vec![weiche_prompt(
        invite_role,
        frischling_role,
        mitspieler_suche,
    )];
    prompts.push(ping_prompt(model, &mut blockers));
    if let Some(mut prompt) = rank_prompt {
        sanitize_carried_over_channel_ids(&mut prompt, model, &mut warnings);
        prompts.push(prompt);
    }
    if mitspieler_suche.is_some() {
        warnings.push(
            "`Ich spiele Deadlock und suche Mitspieler` nutzt `mitspieler-suche` als channel_id-Fallback, damit Discord Optionen ohne Rollen/Kanaele sicher akzeptiert."
                .to_string(),
        );
    }

    Ok(OnboardingBuildOutput {
        config: NativeOnboardingConfig {
            prompts,
            default_channel_ids,
            enabled: true,
            mode: live_config.get("mode").cloned().unwrap_or(live.mode),
        },
        blockers,
        warnings,
    })
}

fn parse_live_onboarding_config(live_config: &Value) -> ServerSyncResult<NativeOnboardingConfig> {
    serde_json::from_value(live_config.clone()).map_err(|err| {
        ServerSyncError::bad_request(format!(
            "Live-Onboarding-Config konnte nicht gelesen werden: {err}"
        ))
    })
}

fn native_onboarding_put_payload(config: &NativeOnboardingConfig) -> NativeOnboardingPutConfig {
    // Discord verlangt das id-Feld auch fuer NEUE Prompts/Optionen
    // (BASE_TYPE_REQUIRED, live verifiziert 2026-07-03); neue Objekte
    // bekommen eindeutige Platzhalter-IDs ("0", "1", ...), die Discord
    // beim PUT durch echte Snowflakes ersetzt.
    let mut placeholder = PlaceholderIdCounter::default();
    NativeOnboardingPutConfig {
        prompts: config
            .prompts
            .iter()
            .map(|prompt| native_onboarding_put_prompt(prompt, &mut placeholder))
            .collect(),
        default_channel_ids: config.default_channel_ids.clone(),
        enabled: config.enabled,
        mode: config.mode.clone(),
    }
}

#[derive(Default)]
struct PlaceholderIdCounter(u64);

impl PlaceholderIdCounter {
    fn fill(&mut self, id: &Option<String>) -> Option<String> {
        id.clone().or_else(|| {
            let next = self.0.to_string();
            self.0 += 1;
            Some(next)
        })
    }
}

fn native_onboarding_put_prompt(
    prompt: &NativeOnboardingPrompt,
    placeholder: &mut PlaceholderIdCounter,
) -> NativeOnboardingPutPrompt {
    NativeOnboardingPutPrompt {
        id: placeholder.fill(&prompt.id),
        prompt_type: prompt.prompt_type,
        title: prompt.title.clone(),
        options: prompt
            .options
            .iter()
            .map(|option| native_onboarding_put_option(option, placeholder))
            .collect(),
        single_select: prompt.single_select,
        required: prompt.required,
        in_onboarding: prompt.in_onboarding,
        extra: put_extra_without_emoji_fields(&prompt.extra),
    }
}

fn native_onboarding_put_option(
    option: &NativeOnboardingOption,
    placeholder: &mut PlaceholderIdCounter,
) -> NativeOnboardingPutOption {
    let (emoji_id, emoji_name, emoji_animated) = put_emoji_fields(option.emoji.as_ref());
    NativeOnboardingPutOption {
        id: placeholder.fill(&option.id),
        title: option.title.clone(),
        description: option.description.clone(),
        emoji_id,
        emoji_name,
        emoji_animated,
        role_ids: option.role_ids.clone(),
        channel_ids: option.channel_ids.clone(),
        extra: put_extra_without_emoji_fields(&option.extra),
    }
}

fn put_emoji_fields(emoji: Option<&Value>) -> (Option<String>, Option<String>, Option<bool>) {
    let Some(emoji) = emoji else {
        return (None, None, None);
    };
    let emoji_id = emoji.get("id").and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|id| id.to_string()))
    });
    let emoji_name = emoji
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let has_emoji = emoji_id.is_some() || emoji_name.is_some();
    let emoji_animated = has_emoji.then(|| {
        emoji
            .get("animated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    (emoji_id, emoji_name, emoji_animated)
}

fn put_extra_without_emoji_fields(extra: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    extra
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "emoji" | "emoji_id" | "emoji_name" | "emoji_animated"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn weiche_prompt(
    invite_role: Option<u64>,
    frischling_role: Option<u64>,
    spieler_suche: Option<u64>,
) -> NativeOnboardingPrompt {
    NativeOnboardingPrompt {
        // Neue Prompts/Optionen lassen `id` im PUT weg. Falls Discord im
        // Live-Apply jemals ein `id`-Feld erzwingt (400), ist der Fallback
        // generierte Platzhalter-Strings; Default bleibt bewusst `None`.
        id: None,
        prompt_type: 0,
        title: "Wo stehst du gerade?".to_string(),
        options: vec![
            NativeOnboardingOption {
                id: None,
                title: "Ich spiele Deadlock und suche Mitspieler".to_string(),
                description: None,
                emoji: Some(json!({ "name": "🎮" })),
                role_ids: Vec::new(),
                // Discord lehnt je nach Guild-Validierung Optionen ohne role_ids
                // UND channel_ids ab; diese neutrale Option zeigt deshalb auf
                // den ohnehin vorgesehenen Default-Kanal `mitspieler-suche`.
                channel_ids: spieler_suche
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default(),
                extra: BTreeMap::new(),
            },
            NativeOnboardingOption {
                id: None,
                title: "Ich brauche noch einen Invite".to_string(),
                // Discord-Limit: Options-Titel max. 50 Zeichen — der volle
                // Konsens-Wortlaut steht in der Description (max. 100).
                description: Some(
                    "Ich hab Deadlock noch nicht — ich brauche einen Invite".to_string(),
                ),
                emoji: Some(json!({ "name": "🔑" })),
                role_ids: invite_role
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default(),
                channel_ids: Vec::new(),
                extra: BTreeMap::new(),
            },
            NativeOnboardingOption {
                id: None,
                title: "Ich bin ganz neu — nehmt mich an die Hand".to_string(),
                description: Some(
                    "Ich bin ganz neu und will's lernen — nehmt mich an die Hand".to_string(),
                ),
                emoji: Some(json!({ "name": "🌱" })),
                role_ids: frischling_role
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default(),
                channel_ids: Vec::new(),
                extra: BTreeMap::new(),
            },
        ],
        single_select: true,
        required: true,
        in_onboarding: true,
        extra: BTreeMap::new(),
    }
}

fn ping_prompt(model: &GuildModel, blockers: &mut Vec<String>) -> NativeOnboardingPrompt {
    let options = [
        (
            "Patchnotes",
            &["Patchnotes Ping Rolle", "Patchnotes"] as &[&str],
        ),
        (
            "Spielersuche",
            &[
                "Spieler-Suche Ping Rolle",
                "Spielersuche Ping Rolle",
                "Spielersuche",
                "Spieler-Suche",
                // Live-Rollenname (Ist-Zustand 2026-07-03).
                "Spieler Suche Ping",
            ],
        ),
        (
            "Events & Turniere",
            &[
                "Events & Turniere Ping Rolle",
                "Events und Turniere Ping Rolle",
                "Events & Turniere",
                "Events Turniere",
                "Turniere",
                "Events",
                // Live existiert keine Events/Turniere-Rolle; Competitive-
                // Turniere/Scrims pingen heute "Grind Custom Ping" (Ist-Zustand
                // 2026-07-03, alte Onboarding-Interessen-Frage).
                "Grind Custom Ping",
            ],
        ),
        (
            "Custom Games",
            &[
                "Custom Games Ping Rolle",
                "Custom Games",
                // Live-Pendant: spaßige Customs (Hide and Seek etc.) pingen
                // heute "Funny Custom Ping" (Ist-Zustand 2026-07-03).
                "Funny Custom Ping",
            ],
        ),
        ("Streams", &["Streams"]),
    ]
    .into_iter()
    .map(|(title, aliases)| NativeOnboardingOption {
        id: None,
        title: title.to_string(),
        description: None,
        emoji: None,
        role_ids: require_role_id(model, aliases, title, blockers)
            .map(|id| vec![id.to_string()])
            .unwrap_or_default(),
        channel_ids: Vec::new(),
        extra: BTreeMap::new(),
    })
    .collect();

    NativeOnboardingPrompt {
        id: None,
        prompt_type: 0,
        title: "Wofür willst du Pings bekommen?".to_string(),
        options,
        single_select: false,
        required: false,
        in_onboarding: true,
        extra: BTreeMap::new(),
    }
}

// Discord erlaubt hart maximal 4 Onboarding-Fragen (TOO_MANY_ONBOARDING_PROMPTS,
// live verifiziert 2026-07-15). Wir publishen drei: Weiche, Pings, Rang.
fn find_rank_prompt(
    live: &NativeOnboardingConfig,
    model: &GuildModel,
) -> Option<NativeOnboardingPrompt> {
    live.prompts
        .iter()
        .find(|prompt| {
            prompt.options.len() == 12
                && prompt
                    .options
                    .iter()
                    .filter(|option| option_points_to_unverified_role(option, model))
                    .count()
                    >= 10
        })
        .cloned()
}

/// Discord behaelt in der Onboarding-Config Kanal-IDs geloeschter Kanaele;
/// uebernommene Prompts muessen vor dem PUT gegen das Live-Modell bereinigt
/// werden, sonst blockt die Apply-Revalidierung dauerhaft an stale IDs.
fn sanitize_carried_over_channel_ids(
    prompt: &mut NativeOnboardingPrompt,
    model: &GuildModel,
    warnings: &mut Vec<String>,
) {
    for option in &mut prompt.options {
        let (kept, dropped): (Vec<String>, Vec<String>) =
            option.channel_ids.drain(..).partition(|channel_id| {
                channel_id
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|id| model.channels.contains_key(&id))
            });
        if !dropped.is_empty() {
            warnings.push(format!(
                "Prompt `{}` Option `{}`: stale Kanal-IDs entfernt: {}",
                prompt.title,
                option.title,
                dropped.join(", ")
            ));
        }
        option.channel_ids = kept;
    }
}

fn option_points_to_unverified_role(option: &NativeOnboardingOption, model: &GuildModel) -> bool {
    option.role_ids.iter().any(|role_id| {
        role_id
            .parse::<u64>()
            .ok()
            .and_then(|id| model.roles.get(&id))
            .is_some_and(|role| role.name.contains("(unverifiziert)"))
    })
}

fn resolve_default_channel_ids(model: &GuildModel, blockers: &mut Vec<String>) -> Vec<String> {
    let mut ids = Vec::new();
    for name in DEFAULT_ONBOARDING_CHANNEL_NAMES {
        match resolve_channel_id(model, name) {
            Ok(Some(id)) => ids.push(id.to_string()),
            Ok(None) => blockers.push(format!(
                "Kanal `{name}` wurde im Live-Guild-Modell nicht gefunden"
            )),
            Err(err) => blockers.push(err),
        }
    }
    ids
}

fn resolve_channel_id(model: &GuildModel, name: &str) -> Result<Option<u64>, String> {
    let expected = normalized_name(name);
    let mut candidates = model
        .channels
        .values()
        .filter(|channel| normalized_name(&channel.name) == expected)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|channel| channel.channel_id);
    match candidates.as_slice() {
        [] => Ok(None),
        [channel] => Ok(Some(channel.channel_id)),
        _ => Err(format!(
            "Kanal `{name}` ist im Live-Guild-Modell mehrdeutig: {}",
            candidates
                .iter()
                .map(|channel| format!("{}:{}", channel.channel_id, channel.name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn require_role_id(
    model: &GuildModel,
    aliases: &[&str],
    label: &str,
    blockers: &mut Vec<String>,
) -> Option<u64> {
    let id = resolve_role_id(model, aliases);
    if id.is_none() {
        blockers.push(format!(
            "Rolle `{label}` wurde im Live-Guild-Modell nicht gefunden"
        ));
    }
    id
}

fn resolve_role_id(model: &GuildModel, aliases: &[&str]) -> Option<u64> {
    let normalized_aliases = aliases
        .iter()
        .map(|alias| normalized_name(alias))
        .collect::<Vec<_>>();
    let mut candidates = model
        .roles
        .values()
        .filter(|role| {
            let role_name = normalized_name(&role.name);
            normalized_aliases.contains(&role_name)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|role| (role.managed, !role.mentionable, role.position));
    candidates.first().map(|role| role.role_id)
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_default_channels_7_5(
    model: &GuildModel,
    default_channel_ids: &[String],
) -> Result<(), String> {
    if default_channel_ids.len() < 7 {
        return Err(format!(
            "Native Onboarding braucht mindestens 7 Default-Kanaele, gefunden {}",
            default_channel_ids.len()
        ));
    }

    let writable = default_channel_ids
        .iter()
        .filter_map(|id| id.parse::<u64>().ok())
        .filter(|channel_id| everyone_can_view_and_send(model, *channel_id))
        .count();
    if writable < 5 {
        return Err(format!(
            "Native Onboarding braucht mindestens 5 Default-Kanaele mit @everyone VIEW+SEND, gefunden {writable}"
        ));
    }
    Ok(())
}

fn validate_onboarding_references(
    model: &GuildModel,
    config: &NativeOnboardingConfig,
) -> Result<(), String> {
    let mut missing = Vec::new();
    validate_channel_ids_present(
        model,
        "default_channel_ids",
        &config.default_channel_ids,
        &mut missing,
    );
    for prompt in &config.prompts {
        for option in &prompt.options {
            let context = format!("Prompt `{}` Option `{}`", prompt.title, option.title);
            validate_role_ids_present(model, &context, &option.role_ids, &mut missing);
            validate_channel_ids_present(model, &context, &option.channel_ids, &mut missing);
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Native-Onboarding-Apply blockiert: Live-Guild-Modell enthaelt referenzierte Rollen/Kanaele nicht mehr:\n- {}",
            missing.join("\n- ")
        ))
    }
}

fn validate_role_ids_present(
    model: &GuildModel,
    context: &str,
    role_ids: &[String],
    missing: &mut Vec<String>,
) {
    for raw in role_ids {
        match raw.parse::<u64>() {
            Ok(id) if model.roles.contains_key(&id) => {}
            Ok(id) => missing.push(format!("{context}: Rolle `{id}` fehlt")),
            Err(_) => missing.push(format!("{context}: Rolle `{raw}` ist keine Discord-ID")),
        }
    }
}

fn validate_channel_ids_present(
    model: &GuildModel,
    context: &str,
    channel_ids: &[String],
    missing: &mut Vec<String>,
) {
    for raw in channel_ids {
        match raw.parse::<u64>() {
            Ok(id) if model.channels.contains_key(&id) => {}
            Ok(id) => missing.push(format!("{context}: Kanal `{id}` fehlt")),
            Err(_) => missing.push(format!("{context}: Kanal `{raw}` ist keine Discord-ID")),
        }
    }
}

fn everyone_can_view_and_send(model: &GuildModel, channel_id: u64) -> bool {
    everyone_permissions_for_channel(model, channel_id)
        .contains(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES)
}

fn everyone_can_view(model: &GuildModel, channel_id: u64) -> bool {
    everyone_permissions_for_channel(model, channel_id).contains(Permissions::VIEW_CHANNEL)
}

fn everyone_permissions_for_channel(model: &GuildModel, channel_id: u64) -> Permissions {
    let mut permissions = model
        .roles
        .get(&model.guild_id)
        .or_else(|| model.roles.values().find(|role| role.name == "@everyone"))
        .map(|role| Permissions::from_bits_truncate(role.permissions_bitmask))
        .unwrap_or_default();

    if let Some(parent_id) = model
        .channels
        .get(&channel_id)
        .and_then(|channel| channel.parent_category_id)
    {
        apply_everyone_overwrite(model, parent_id, &mut permissions);
    }
    apply_everyone_overwrite(model, channel_id, &mut permissions);
    permissions
}

fn apply_everyone_overwrite(model: &GuildModel, channel_id: u64, permissions: &mut Permissions) {
    let key = dl_server_as_code::OverwriteKey {
        channel_id,
        target_kind: TargetKind::Role,
        target_id: model.guild_id,
    };
    if let Some(overwrite) = model.overwrites.get(&key) {
        *permissions &= !Permissions::from_bits_truncate(overwrite.deny_bits);
        *permissions |= Permissions::from_bits_truncate(overwrite.allow_bits);
    }
}

fn onboarding_diff(
    guild_id: u64,
    live_config: &Value,
    desired_config: &NativeOnboardingConfig,
) -> ServerSyncResult<ServerDiff> {
    let actual_config = parse_live_onboarding_config(live_config)?;
    let desired_value = json!({
        "name": ONBOARDING_DIFF_MESSAGE_KEY,
        "config": desired_config,
    });
    let actual_value = json!({
        "name": ONBOARDING_DIFF_MESSAGE_KEY,
        "config": actual_config,
    });
    let mut fields = Vec::new();
    for field in ["prompts", "default_channel_ids", "enabled", "mode"] {
        let desired = serde_json::to_value(desired_config)?
            .get(field)
            .cloned()
            .unwrap_or(Value::Null);
        let actual = serde_json::to_value(&actual_config)?
            .get(field)
            .cloned()
            .unwrap_or(Value::Null);
        if desired != actual {
            fields.push(FieldDiff {
                field: field.to_string(),
                desired,
                actual,
            });
        }
    }
    let changes = vec![DiffChange {
        object: ObjectRef {
            kind: ObjectKind::BotMessage,
            guild_id,
            object_id: ONBOARDING_DIFF_OBJECT_ID,
            channel_id: None,
            target_kind: None,
            target_id: None,
            message_key: Some(ONBOARDING_DIFF_MESSAGE_KEY.to_string()),
        },
        action: DiffAction::Update,
        fields,
        desired: Some(desired_value),
        actual: Some(actual_value),
    }];
    Ok(ServerDiff {
        guild_id,
        changes,
        filtered: Vec::new(),
        blocked: Vec::new(),
    })
}

fn serverguide_diff(
    guild_id: u64,
    live_config: &Value,
    desired_config: &ServerGuideConfig,
) -> ServerSyncResult<ServerDiff> {
    let actual_config: ServerGuideConfig = serde_json::from_value(live_config.clone())?;
    let actual_config_value = serde_json::to_value(&actual_config)?;
    let desired_value = json!({
        "name": SERVER_GUIDE_DIFF_MESSAGE_KEY,
        "config": desired_config,
    });
    let actual_value = json!({
        "name": SERVER_GUIDE_DIFF_MESSAGE_KEY,
        "config": actual_config_value.clone(),
    });
    let desired_config_value = serde_json::to_value(desired_config)?;
    let mut fields = Vec::new();
    for field in [
        "enabled",
        "welcome_message",
        "new_member_actions",
        "resource_channels",
    ] {
        let desired = desired_config_value
            .get(field)
            .cloned()
            .unwrap_or(Value::Null);
        let actual = actual_config_value
            .get(field)
            .cloned()
            .unwrap_or(Value::Null);
        if desired != actual {
            fields.push(FieldDiff {
                field: field.to_string(),
                desired,
                actual,
            });
        }
    }
    Ok(ServerDiff {
        guild_id,
        changes: vec![DiffChange {
            object: ObjectRef {
                kind: ObjectKind::BotMessage,
                guild_id,
                object_id: SERVER_GUIDE_DIFF_OBJECT_ID,
                channel_id: None,
                target_kind: None,
                target_id: None,
                message_key: Some(SERVER_GUIDE_DIFF_MESSAGE_KEY.to_string()),
            },
            action: DiffAction::Update,
            fields,
            desired: Some(desired_value),
            actual: Some(actual_value),
        }],
        filtered: Vec::new(),
        blocked: Vec::new(),
    })
}

fn onboarding_human_summary(diff: &ServerDiff, config: &NativeOnboardingConfig) -> String {
    let prompt_titles = config
        .prompts
        .iter()
        .map(|prompt| prompt.title.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "Onboarding-Diff fuer Guild {}: {} Aenderung(en). Prompts: {}",
        diff.guild_id,
        diff.changes.len(),
        prompt_titles
    )
}

fn serverguide_human_summary(diff: &ServerDiff, config: &ServerGuideConfig) -> String {
    let actions = config
        .new_member_actions
        .iter()
        .map(|action| action.title.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "Server-Guide-Diff fuer Guild {}: {} Aenderung(en). Actions: {}",
        diff.guild_id,
        diff.changes.len(),
        actions
    )
}

fn onboarding_blocker_summary(blockers: &[String]) -> String {
    format!(
        "Onboarding-Preview blockiert: {} Blocker.\n- {}",
        blockers.len(),
        blockers.join("\n- ")
    )
}

fn serverguide_blocker_summary(blockers: &[String]) -> String {
    format!(
        "Server-Guide-Preview blockiert: {} Blocker.\n- {}",
        blockers.len(),
        blockers.join("\n- ")
    )
}

#[derive(Debug)]
struct StoredOnboardingPreview {
    guild_id: u64,
    diff_hash: String,
    config: NativeOnboardingConfig,
}

#[derive(Debug)]
struct StoredServerGuidePreview {
    guild_id: u64,
    diff_hash: String,
    config: ServerGuideConfig,
}

async fn persist_onboarding_preview(
    pool: &PgPool,
    snapshot_id: Option<i64>,
    diff: &ServerDiff,
    human_summary: &str,
    created_by_user_id: Option<u64>,
) -> ServerSyncResult<dl_server_as_code::db::DiffPreview> {
    let diff_json = serde_json::to_string(diff)?;
    let diff_hash = sha256_hex(&serde_json::to_vec(diff)?);
    let (preview_id, created_snapshot_id, created_at): (i64, Option<i64>, DateTime<Utc>) =
        sqlx::query_as(
            "INSERT INTO server_config.diff_previews
         (guild_id, snapshot_id, diff_hash, diff_json, human_summary, created_by_user_id)
         VALUES ($1, $2, $3, $4::text::jsonb, $5, $6)
         ON CONFLICT (guild_id, diff_hash)
         DO UPDATE SET snapshot_id = EXCLUDED.snapshot_id,
                       diff_json = EXCLUDED.diff_json,
                       human_summary = EXCLUDED.human_summary,
                       created_by_user_id = EXCLUDED.created_by_user_id,
                       created_at = now()
         RETURNING preview_id, snapshot_id, created_at",
        )
        .bind(id_to_i64(diff.guild_id)?)
        .bind(snapshot_id)
        .bind(&diff_hash)
        .bind(diff_json)
        .bind(human_summary)
        .bind(created_by_user_id.map(id_to_i64).transpose()?)
        .fetch_one(pool)
        .await?;

    Ok(dl_server_as_code::db::DiffPreview {
        preview_id,
        guild_id: diff.guild_id,
        snapshot_id: created_snapshot_id,
        created_at,
        diff_hash,
        human_summary: human_summary.to_string(),
        diff: diff.clone(),
    })
}

async fn load_onboarding_preview(
    pool: &PgPool,
    preview_id: i64,
    guild_id: u64,
) -> ServerSyncResult<StoredOnboardingPreview> {
    let row = sqlx::query(
        "SELECT guild_id,
                diff_hash,
                diff_json::text AS diff_json,
                (now() - created_at) > ($6::double precision * interval '1 minute') AS preview_expired
           FROM server_config.diff_previews
          WHERE preview_id = $1
            AND guild_id = $2
            AND applied_at IS NULL
            AND EXISTS (
                SELECT 1
                  FROM jsonb_array_elements(COALESCE(diff_json->'changes', '[]'::jsonb)) AS elem(change)
                 WHERE elem.change->'object'->>'kind' = $3
                   AND elem.change->'object'->>'message_key' = $4
                   AND elem.change->'object'->>'object_id' = $5
            )",
    )
    .bind(preview_id)
    .bind(id_to_i64(guild_id)?)
    .bind(ObjectKind::BotMessage.as_db())
    .bind(ONBOARDING_DIFF_MESSAGE_KEY)
    .bind(ONBOARDING_DIFF_OBJECT_ID.to_string())
    .bind(PREVIEW_MAX_AGE_MINUTES as f64)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ServerSyncError::bad_request(format!(
            "Onboarding-Preview {preview_id} nicht gefunden, guild-fremd, bereits angewendet oder keine Native-Onboarding-Preview"
        ))
    })?;

    ensure_preview_not_expired(row.try_get("preview_expired")?)?;
    let diff_json: String = row.try_get("diff_json")?;
    let diff: ServerDiff = serde_json::from_str(&diff_json)?;
    let stored_hash: String = row.try_get("diff_hash")?;
    let recomputed = sha256_hex(&serde_json::to_vec(&diff)?);
    if stored_hash != recomputed {
        return Err(ServerSyncError::bad_request(format!(
            "Onboarding-Preview {preview_id} Hash-Mismatch: gespeichert {stored_hash}, berechnet {recomputed}"
        )));
    }
    let config = extract_onboarding_config_from_diff(&diff, guild_id)?;
    Ok(StoredOnboardingPreview {
        guild_id: i64_to_u64(row.try_get::<i64, _>("guild_id")?)?,
        diff_hash: stored_hash,
        config,
    })
}

async fn load_serverguide_preview(
    pool: &PgPool,
    preview_id: i64,
    guild_id: u64,
) -> ServerSyncResult<StoredServerGuidePreview> {
    let row = sqlx::query(
        "SELECT guild_id,
                diff_hash,
                diff_json::text AS diff_json,
                (now() - created_at) > ($6::double precision * interval '1 minute') AS preview_expired
           FROM server_config.diff_previews
          WHERE preview_id = $1
            AND guild_id = $2
            AND applied_at IS NULL
            AND EXISTS (
                SELECT 1
                  FROM jsonb_array_elements(COALESCE(diff_json->'changes', '[]'::jsonb)) AS elem(change)
                 WHERE elem.change->'object'->>'kind' = $3
                   AND elem.change->'object'->>'message_key' = $4
                   AND elem.change->'object'->>'object_id' = $5
            )",
    )
    .bind(preview_id)
    .bind(id_to_i64(guild_id)?)
    .bind(ObjectKind::BotMessage.as_db())
    .bind(SERVER_GUIDE_DIFF_MESSAGE_KEY)
    .bind(SERVER_GUIDE_DIFF_OBJECT_ID.to_string())
    .bind(PREVIEW_MAX_AGE_MINUTES as f64)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ServerSyncError::bad_request(format!(
            "Server-Guide-Preview {preview_id} nicht gefunden, guild-fremd, bereits angewendet oder keine Server-Guide-Preview"
        ))
    })?;

    ensure_preview_not_expired(row.try_get("preview_expired")?)?;
    let diff_json: String = row.try_get("diff_json")?;
    let diff: ServerDiff = serde_json::from_str(&diff_json)?;
    let stored_hash: String = row.try_get("diff_hash")?;
    let recomputed = sha256_hex(&serde_json::to_vec(&diff)?);
    if stored_hash != recomputed {
        return Err(ServerSyncError::bad_request(format!(
            "Server-Guide-Preview {preview_id} Hash-Mismatch: gespeichert {stored_hash}, berechnet {recomputed}"
        )));
    }
    let config = extract_serverguide_config_from_diff(&diff, guild_id)?;
    Ok(StoredServerGuidePreview {
        guild_id: i64_to_u64(row.try_get::<i64, _>("guild_id")?)?,
        diff_hash: stored_hash,
        config,
    })
}

fn extract_onboarding_config_from_diff(
    diff: &ServerDiff,
    expected_guild_id: u64,
) -> ServerSyncResult<NativeOnboardingConfig> {
    if diff.guild_id != expected_guild_id {
        return Err(ServerSyncError::bad_request(format!(
            "Preview gehoert zu Guild {}, erwartet {expected_guild_id}",
            diff.guild_id
        )));
    }
    let desired = diff
        .changes
        .iter()
        .find(|change| is_native_onboarding_change(change, expected_guild_id))
        .and_then(|change| change.desired.as_ref())
        .ok_or_else(|| {
            ServerSyncError::bad_request("Preview enthaelt keine Native-Onboarding-Zielconfig")
        })?;
    serde_json::from_value(desired["config"].clone()).map_err(|err| {
        ServerSyncError::bad_request(format!("Native-Onboarding-Zielconfig ist ungueltig: {err}"))
    })
}

fn extract_serverguide_config_from_diff(
    diff: &ServerDiff,
    expected_guild_id: u64,
) -> ServerSyncResult<ServerGuideConfig> {
    if diff.guild_id != expected_guild_id {
        return Err(ServerSyncError::bad_request(format!(
            "Preview gehoert zu Guild {}, erwartet {expected_guild_id}",
            diff.guild_id
        )));
    }
    let desired = diff
        .changes
        .iter()
        .find(|change| is_serverguide_change(change, expected_guild_id))
        .and_then(|change| change.desired.as_ref())
        .ok_or_else(|| {
            ServerSyncError::bad_request("Preview enthaelt keine Server-Guide-Zielconfig")
        })?;
    serde_json::from_value(desired["config"].clone()).map_err(|err| {
        ServerSyncError::bad_request(format!("Server-Guide-Zielconfig ist ungueltig: {err}"))
    })
}

fn is_native_onboarding_change(change: &DiffChange, expected_guild_id: u64) -> bool {
    change.object.kind == ObjectKind::BotMessage
        && change.object.guild_id == expected_guild_id
        && change.object.object_id == ONBOARDING_DIFF_OBJECT_ID
        && change.object.message_key.as_deref() == Some(ONBOARDING_DIFF_MESSAGE_KEY)
}

fn is_serverguide_change(change: &DiffChange, expected_guild_id: u64) -> bool {
    change.object.kind == ObjectKind::BotMessage
        && change.object.guild_id == expected_guild_id
        && change.object.object_id == SERVER_GUIDE_DIFF_OBJECT_ID
        && change.object.message_key.as_deref() == Some(SERVER_GUIDE_DIFF_MESSAGE_KEY)
}

struct OnboardingApplyRunInsert<'a> {
    pool: &'a PgPool,
    preview_id: i64,
    guild_id: u64,
    confirmed_hash: &'a str,
    requested_by_user_id: Option<u64>,
    dry_run: bool,
    status: &'a str,
    result: Value,
    error_text: Option<&'a str>,
}

async fn insert_onboarding_apply_run(input: OnboardingApplyRunInsert<'_>) -> ServerSyncResult<i64> {
    let apply_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO server_config.apply_runs
         (preview_id, guild_id, confirmed_diff_hash, requested_by_user_id, dry_run, status, result_json, error_text, finished_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7::text::jsonb, $8, CASE WHEN $6 = 'running' THEN NULL ELSE now() END)
         RETURNING apply_run_id",
    )
    .bind(input.preview_id)
    .bind(id_to_i64(input.guild_id)?)
    .bind(input.confirmed_hash)
    .bind(input.requested_by_user_id.map(id_to_i64).transpose()?)
    .bind(input.dry_run)
    .bind(input.status)
    .bind(input.result.to_string())
    .bind(input.error_text)
    .fetch_one(input.pool)
    .await?;
    Ok(apply_run_id)
}

async fn finish_onboarding_apply_run(
    pool: &PgPool,
    apply_run_id: i64,
    status: &str,
    result: Value,
    error_text: Option<&str>,
) -> ServerSyncResult<()> {
    sqlx::query(
        "UPDATE server_config.apply_runs
            SET status = $2,
                result_json = $3::text::jsonb,
                error_text = $4,
                finished_at = now()
          WHERE apply_run_id = $1",
    )
    .bind(apply_run_id)
    .bind(status)
    .bind(result.to_string())
    .bind(error_text)
    .execute(pool)
    .await?;
    Ok(())
}

fn collect_unique_by<T, K, F>(
    specs: &[T],
    label: &str,
    mut key_fn: F,
) -> ServerSyncResult<BTreeMap<K, T>>
where
    T: Clone,
    K: Clone + Ord + std::fmt::Debug,
    F: FnMut(&T) -> ServerSyncResult<K>,
{
    let mut out = BTreeMap::new();
    for spec in specs {
        let key = key_fn(spec)?;
        if out.insert(key.clone(), spec.clone()).is_some() {
            return Err(ServerSyncError::bad_request(format!(
                "Rollback-Artefakt enthaelt doppelten {label}-Key {key:?}"
            )));
        }
    }
    Ok(out)
}

fn ensure_spec_guild(
    label: &str,
    object_id: u64,
    actual_guild_id: u64,
    expected_guild_id: u64,
) -> ServerSyncResult<()> {
    if actual_guild_id == expected_guild_id {
        return Ok(());
    }
    Err(ServerSyncError::bad_request(format!(
        "Rollback-Artefakt enthaelt {label} {object_id} fuer Guild {actual_guild_id}, erwartet {expected_guild_id}"
    )))
}

fn id_to_i64(value: u64) -> ServerSyncResult<i64> {
    i64::try_from(value)
        .map_err(|_| ServerSyncError::bad_request("Discord-ID passt nicht in BIGINT"))
}

fn i64_to_u64(value: i64) -> ServerSyncResult<u64> {
    u64::try_from(value)
        .map_err(|_| ServerSyncError::bad_request("BIGINT passt nicht in Discord-ID"))
}

fn ensure_preview_not_expired(preview_expired: bool) -> ServerSyncResult<()> {
    if preview_expired {
        return Err(ServerSyncError::bad_request(
            "Preview abgelaufen, bitte neu diffen",
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_json_value(value: &Value) -> ServerSyncResult<String> {
    let canonical = canonical_json_value(value);
    Ok(sha256_hex(&serde_json::to_vec(&canonical)?))
}

fn verify_rollback_artifact(
    loaded_id: i64,
    stored_hash: &str,
    artifact_value: Value,
    expected_guild_id: u64,
) -> ServerSyncResult<RollbackArtifact> {
    let version = artifact_value
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if version == ROLLBACK_VERSION_V1 {
        return Err(ServerSyncError::bad_request(format!(
            "Rollback-Artefakt {loaded_id} ist v1 und enthaelt keine Diff-Ausnahmen; bitte neuen v2-Rollback-Export erstellen"
        )));
    }
    if version != ROLLBACK_VERSION {
        return Err(ServerSyncError::bad_request(format!(
            "Rollback-Artefakt {loaded_id} hat nicht unterstuetzte Version {version:?}; erwartet {ROLLBACK_VERSION}"
        )));
    }

    let actual_hash = sha256_json_value(&artifact_value)?;
    if actual_hash != stored_hash {
        return Err(ServerSyncError::bad_request(format!(
            "Rollback-Artefakt {loaded_id} Hash-Mismatch: gespeichert {stored_hash}, berechnet {actual_hash}"
        )));
    }

    let artifact: RollbackArtifact = serde_json::from_value(artifact_value).map_err(|err| {
        ServerSyncError::bad_request(format!(
            "Rollback-Artefakt {loaded_id} ist kein lesbares v2-Artefakt: {err}"
        ))
    })?;
    if artifact.guild_id != expected_guild_id
        || artifact.structure_snapshot.guild_id != expected_guild_id
    {
        return Err(ServerSyncError::bad_request(format!(
            "Rollback-Artefakt {loaded_id} gehoert nicht zu Guild {expected_guild_id}"
        )));
    }
    artifact.structure_snapshot.to_model()?;
    Ok(artifact)
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(value) = map.get(key) {
                    out.insert(key.clone(), canonical_json_value(value));
                }
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

fn rollback_filename(rollback_export_id: i64, snapshot_id: i64) -> String {
    format!("serversync-rollback-{rollback_export_id}-snapshot-{snapshot_id}.json")
}

pub fn command_spec() -> CommandSpec {
    CommandSpec {
        definition: json!({
            "name": "serversync",
            "description": "Owner-Werkzeug fuer Server-as-Code",
            "options": [
                {
                    "type": 1,
                    "name": "snapshot",
                    "description": "Live-Struktur-Snapshot importieren"
                },
                {
                    "type": 1,
                    "name": "rollback-export",
                    "description": "Rollback-Artefakt mit Member-Rollen und Onboarding exportieren"
                },
                {
                    "type": 1,
                    "name": "diff",
                    "description": "Dry-Run-Diff gegen das Soll-Modell erzeugen"
                },
                {
                    "type": 1,
                    "name": "restore",
                    "description": "Rollback-Artefakt als hash-gated Restore-Preview erzeugen",
                    "options": [
                        {
                            "type": 4,
                            "name": "rollback_export_id",
                            "description": "Rollback-Export-ID; fehlt = letzter gueltiger Export",
                            "required": false
                        }
                    ]
                },
                {
                    "type": 1,
                    "name": "onboarding-preview",
                    "description": "Native-Onboarding-Diff erzeugen"
                },
                {
                    "type": 1,
                    "name": "serverguide-preview",
                    "description": "serverguide-preview"
                },
                {
                    "type": 1,
                    "name": "archive-enable",
                    "description": "archive-enable"
                },
                {
                    "type": 1,
                    "name": "archive-disable",
                    "description": "archive-disable"
                },
                {
                    "type": 1,
                    "name": "regelwerk-publish",
                    "description": "Regelwerk-Publish",
                    "options": [
                        {
                            "type": 5,
                            "name": "confirm",
                            "description": "true = live; false = dry-run",
                            "required": false
                        }
                    ]
                },
                {
                    "type": 1,
                    "name": "onboarding-apply",
                    "description": "Native-Onboarding-Preview hash-gated anwenden; ohne confirm nur Dry-Run",
                    "options": [
                        {
                            "type": 4,
                            "name": "preview_id",
                            "description": "Diff-Preview-ID",
                            "required": true
                        },
                        {
                            "type": 3,
                            "name": "hash",
                            "description": "Bestaetigter diff_hash",
                            "required": true
                        },
                        {
                            "type": 5,
                            "name": "confirm",
                            "description": "true = echter Apply; fehlt/false = Dry-Run",
                            "required": false
                        }
                    ]
                },
                {
                    "type": 1,
                    "name": "serverguide-apply",
                    "description": "serverguide-apply",
                    "options": [
                        {
                            "type": 4,
                            "name": "preview_id",
                            "description": "preview_id",
                            "required": true
                        },
                        {
                            "type": 3,
                            "name": "hash",
                            "description": "diff_hash",
                            "required": true
                        },
                        {
                            "type": 5,
                            "name": "confirm",
                            "description": "true = live; false = dry-run",
                            "required": false
                        }
                    ]
                },
                {
                    "type": 1,
                    "name": "apply",
                    "description": "Preview hash-gated anwenden; ohne confirm nur Dry-Run",
                    "options": [
                        {
                            "type": 4,
                            "name": "preview_id",
                            "description": "Diff-Preview-ID",
                            "required": true
                        },
                        {
                            "type": 3,
                            "name": "hash",
                            "description": "Bestaetigter diff_hash",
                            "required": true
                        },
                        {
                            "type": 5,
                            "name": "confirm",
                            "description": "true = echter Apply; fehlt/false = Dry-Run",
                            "required": false
                        }
                    ]
                }
            ],
        }),
    }
}

pub fn register_commands(
    router: &mut dl_discord::InteractionRouter,
    service: SharedServerSync,
    owner_id: Option<u64>,
) {
    let handler = Arc::new(ServerSyncCommand { service, owner_id });
    let spec = command_spec();
    router.on_command("serversync snapshot", spec.clone(), handler.clone());
    router.on_command("serversync rollback-export", spec.clone(), handler.clone());
    router.on_command("serversync diff", spec.clone(), handler.clone());
    router.on_command("serversync restore", spec.clone(), handler.clone());
    router.on_command(
        "serversync onboarding-preview",
        spec.clone(),
        handler.clone(),
    );
    router.on_command(
        "serversync serverguide-preview",
        spec.clone(),
        handler.clone(),
    );
    router.on_command("serversync archive-enable", spec.clone(), handler.clone());
    router.on_command("serversync archive-disable", spec.clone(), handler.clone());
    router.on_command(
        "serversync regelwerk-publish",
        spec.clone(),
        handler.clone(),
    );
    router.on_command("serversync onboarding-apply", spec.clone(), handler.clone());
    router.on_command(
        "serversync serverguide-apply",
        spec.clone(),
        handler.clone(),
    );
    router.on_command("serversync apply", spec, handler);
}

pub fn register_regelwerk_components(router: &mut dl_discord::InteractionRouter) {
    regelwerk_publish::register_components(router);
}

pub fn register_faq_components(router: &mut dl_discord::InteractionRouter) {
    faq_publish::register_components(router);
}

struct ServerSyncCommand {
    service: SharedServerSync,
    owner_id: Option<u64>,
}

#[async_trait]
impl dl_discord::InteractionHandler for ServerSyncCommand {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.guild_id != self.service.guild_id()
            || !master::is_owner(interaction.user_id, self.owner_id)
        {
            return BridgeReply::ephemeral_text("Keine Berechtigung.");
        }

        match interaction.command.as_str() {
            "serversync snapshot" => {
                command_snapshot(self.service.as_ref(), interaction.user_id).await
            }
            "serversync rollback-export" => {
                command_rollback_export(self.service.as_ref(), interaction.user_id).await
            }
            "serversync diff" => command_diff(self.service.as_ref(), interaction.user_id).await,
            "serversync restore" => command_restore(self.service.as_ref(), &interaction).await,
            "serversync onboarding-preview" => {
                command_onboarding_preview(self.service.as_ref(), interaction.user_id).await
            }
            "serversync serverguide-preview" => {
                command_serverguide_preview(self.service.as_ref(), interaction.user_id).await
            }
            "serversync archive-enable" => command_archive_flag(self.service.as_ref(), true).await,
            "serversync archive-disable" => {
                command_archive_flag(self.service.as_ref(), false).await
            }
            "serversync regelwerk-publish" => {
                command_regelwerk_publish(self.service.as_ref(), &interaction).await
            }
            "serversync onboarding-apply" => {
                command_onboarding_apply(self.service.as_ref(), &interaction).await
            }
            "serversync serverguide-apply" => {
                command_serverguide_apply(self.service.as_ref(), &interaction).await
            }
            "serversync apply" => command_apply(self.service.as_ref(), &interaction).await,
            _ => BridgeReply::ephemeral_text("Unbekannter Server-Sync-Befehl."),
        }
    }
}

async fn command_snapshot(service: &dyn ServerSyncOps, user_id: u64) -> BridgeReply {
    match service.snapshot(Some(user_id)).await {
        Ok(output) => BridgeReply::ephemeral_text(format!(
            "Snapshot importiert.\nsnapshot_id: {}\nKategorien: {} | Kanaele: {} | Rollen: {} | Overwrites: {}",
            output.snapshot_id, output.categories, output.channels, output.roles, output.overwrites
        )),
        Err(err) => command_error(err),
    }
}

async fn command_rollback_export(service: &dyn ServerSyncOps, user_id: u64) -> BridgeReply {
    match service.rollback_export(Some(user_id)).await {
        Ok(output) => {
            let (attachments, notice) = attachment_or_db_notice(
                output.filename,
                output.artifact_text.into_bytes(),
                "Artefakt",
                output.rollback_export_id,
            );
            let content = format!(
                "Rollback-Export erstellt.\nrollback_export_id: {}\nsnapshot_id: {}\nartifact_hash: {}\nMember: {} | Rollen-Zuweisungen: {}",
                output.rollback_export_id,
                output.snapshot_id,
                output.artifact_hash,
                output.members,
                output.member_role_edges
            );
            BridgeReply {
                content: Some(content + &notice),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_diff(service: &dyn ServerSyncOps, user_id: u64) -> BridgeReply {
    match service.diff(Some(user_id)).await {
        Ok(output) => {
            let warnings = warning_text(&output.warnings);
            let text = format!(
                "Diff-Preview erstellt.\npreview_id: {}\ndiff_hash: {}\n{}{}",
                output.preview_id, output.diff_hash, warnings, output.human_summary
            );
            let (attachments, notice) = attachment_or_db_notice(
                format!("serversync-diff-{}.json", output.preview_id),
                output.diff_text.into_bytes(),
                "Diff",
                output.preview_id,
            );
            BridgeReply {
                content: Some(truncate_discord(&(text + &notice))),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_restore(
    service: &dyn ServerSyncOps,
    interaction: &BridgeInteraction,
) -> BridgeReply {
    let rollback_export_id = match option_i64_optional(&interaction.options, "rollback_export_id") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    match service
        .restore(rollback_export_id, Some(interaction.user_id))
        .await
    {
        Ok(output) => {
            let warnings = warning_text(&output.warnings);
            let text = format!(
                "Restore-Preview erstellt.\nrollback_export_id: {}\npreview_id: {}\nartifact_hash: {}\ndiff_hash: {}\n{}{}",
                output.rollback_export_id,
                output.preview_id,
                output.artifact_hash,
                output.diff_hash,
                warnings,
                output.human_summary
            );
            let (attachments, notice) = attachment_or_db_notice(
                format!("serversync-restore-diff-{}.json", output.preview_id),
                output.diff_text.into_bytes(),
                "Diff",
                output.preview_id,
            );
            BridgeReply {
                content: Some(truncate_discord(&(text + &notice))),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_onboarding_preview(service: &dyn ServerSyncOps, user_id: u64) -> BridgeReply {
    match service.onboarding_preview(Some(user_id)).await {
        Ok(output) => {
            let warnings = warning_text(&output.warnings);
            let blockers = warning_text(&output.blockers);
            let hash_text = output
                .diff_hash
                .as_deref()
                .map(|hash| format!("diff_hash: {hash}\n"))
                .unwrap_or_default();
            let preview_text = output
                .preview_id
                .map(|id| format!("preview_id: {id}\n"))
                .unwrap_or_default();
            let text = format!(
                "Onboarding-Preview.\n{}{}{}{}{}",
                preview_text, hash_text, blockers, warnings, output.human_summary
            );
            let (attachments, notice) = attachment_or_db_notice(
                format!(
                    "serversync-onboarding-diff-{}.json",
                    output.preview_id.unwrap_or_default()
                ),
                output.diff_text.into_bytes(),
                "Diff",
                output.preview_id.unwrap_or_default(),
            );
            BridgeReply {
                content: Some(truncate_discord(&(text + &notice))),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_serverguide_preview(service: &dyn ServerSyncOps, user_id: u64) -> BridgeReply {
    match service.serverguide_preview(Some(user_id)).await {
        Ok(output) => {
            let warnings = warning_text(&output.warnings);
            let blockers = warning_text(&output.blockers);
            let hash_text = output
                .diff_hash
                .as_deref()
                .map(|hash| format!("diff_hash: {hash}\n"))
                .unwrap_or_default();
            let preview_text = output
                .preview_id
                .map(|id| format!("preview_id: {id}\n"))
                .unwrap_or_default();
            let text = format!(
                "serverguide_preview\n{}{}{}{}{}",
                preview_text, hash_text, blockers, warnings, output.human_summary
            );
            let (attachments, notice) = attachment_or_db_notice(
                format!(
                    "serversync-serverguide-diff-{}.json",
                    output.preview_id.unwrap_or_default()
                ),
                output.diff_text.into_bytes(),
                "Diff",
                output.preview_id.unwrap_or_default(),
            );
            BridgeReply {
                content: Some(truncate_discord(&(text + &notice))),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_archive_flag(service: &dyn ServerSyncOps, enabled: bool) -> BridgeReply {
    let result = if enabled {
        service.archive_enable().await
    } else {
        service.archive_disable().await
    };
    match result {
        Ok(output) => BridgeReply::ephemeral_text(format!("archive_enabled: {}", output.enabled)),
        Err(err) => command_error(err),
    }
}

async fn command_regelwerk_publish(
    service: &dyn ServerSyncOps,
    interaction: &BridgeInteraction,
) -> BridgeReply {
    let confirm = option_bool(&interaction.options, "confirm").unwrap_or(false);
    match service.regelwerk_publish(confirm).await {
        Ok(output) => BridgeReply::ephemeral_text(format!(
            "regelwerk_publish\nmode: {}\nthreads_found: {}\nthreads_deleted: {}\nbot_messages_found: {}\nbot_messages_deleted: {}\nposted_message_id: {:?}\nedited_message_id: {:?}",
            if output.dry_run { "dry-run" } else { "live" },
            output.threads_found,
            output.threads_deleted,
            output.bot_messages_found,
            output.bot_messages_deleted,
            output.posted_message_id,
            output.edited_message_id
        )),
        Err(err) => command_error(err),
    }
}

async fn command_onboarding_apply(
    service: &dyn ServerSyncOps,
    interaction: &BridgeInteraction,
) -> BridgeReply {
    let preview_id = match option_i64(&interaction.options, "preview_id") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    let hash = match option_string(&interaction.options, "hash") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    let confirm = option_bool(&interaction.options, "confirm").unwrap_or(false);
    match service
        .onboarding_apply(preview_id, hash, confirm, Some(interaction.user_id))
        .await
    {
        Ok(output) => {
            let (attachments, notice) = attachment_or_db_notice(
                format!("serversync-onboarding-apply-{}.json", output.apply_run_id),
                output.details_text.into_bytes(),
                "Apply-Report",
                output.apply_run_id,
            );
            let content = format!(
                "Onboarding-Apply.\npreview_id: {}\napply_run_id: {}\nModus: {}\napplied: {} | skipped: {} | failed: {}",
                output.preview_id,
                output.apply_run_id,
                if output.dry_run { "dry-run" } else { "live" },
                output.applied,
                output.skipped,
                output.failed
            );
            BridgeReply {
                content: Some(content + &notice),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_serverguide_apply(
    service: &dyn ServerSyncOps,
    interaction: &BridgeInteraction,
) -> BridgeReply {
    let preview_id = match option_i64(&interaction.options, "preview_id") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    let hash = match option_string(&interaction.options, "hash") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    let confirm = option_bool(&interaction.options, "confirm").unwrap_or(false);
    match service
        .serverguide_apply(preview_id, hash, confirm, Some(interaction.user_id))
        .await
    {
        Ok(output) => {
            let (attachments, notice) = attachment_or_db_notice(
                format!("serversync-serverguide-apply-{}.json", output.apply_run_id),
                output.details_text.into_bytes(),
                "Apply-Report",
                output.apply_run_id,
            );
            let content = format!(
                "serverguide_apply\npreview_id: {}\napply_run_id: {}\nmode: {}\napplied: {} | skipped: {} | failed: {}",
                output.preview_id,
                output.apply_run_id,
                if output.dry_run { "dry-run" } else { "live" },
                output.applied,
                output.skipped,
                output.failed
            );
            BridgeReply {
                content: Some(content + &notice),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

async fn command_apply(
    service: &dyn ServerSyncOps,
    interaction: &BridgeInteraction,
) -> BridgeReply {
    let preview_id = match option_i64(&interaction.options, "preview_id") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    let hash = match option_string(&interaction.options, "hash") {
        Ok(value) => value,
        Err(err) => return command_error(err),
    };
    let confirm = option_bool(&interaction.options, "confirm").unwrap_or(false);
    match service
        .apply(preview_id, hash, confirm, Some(interaction.user_id))
        .await
    {
        Ok(output) => {
            let (attachments, notice) = attachment_or_db_notice(
                format!("serversync-apply-{}.json", output.apply_run_id),
                output.details_text.into_bytes(),
                "Apply-Report",
                output.apply_run_id,
            );
            let content = format!(
                "Apply-Report.\npreview_id: {}\napply_run_id: {}\nModus: {}\napplied: {} | skipped: {} | failed: {}",
                output.preview_id,
                output.apply_run_id,
                if output.dry_run { "dry-run" } else { "live" },
                output.applied,
                output.skipped,
                output.failed
            );
            BridgeReply {
                content: Some(content + &notice),
                ephemeral: true,
                attachments,
                ..BridgeReply::default()
            }
        }
        Err(err) => command_error(err),
    }
}

fn command_error(error: ServerSyncError) -> BridgeReply {
    BridgeReply::ephemeral_text(format!("Server-Sync fehlgeschlagen: {error}"))
}

fn attachment_or_db_notice(
    filename: String,
    data: Vec<u8>,
    db_label: &str,
    db_id: i64,
) -> (Vec<BridgeAttachment>, String) {
    if data.len() <= MAX_BRIDGE_ATTACHMENT_BYTES {
        return (vec![BridgeAttachment { filename, data }], String::new());
    }
    (
        Vec::new(),
        format!("\n{db_label} liegt in der DB (id {db_id}); Attachment war groesser als 10 MiB."),
    )
}

fn warning_text(warnings: &[String]) -> String {
    if warnings.is_empty() {
        String::new()
    } else {
        format!("Warnings:\n- {}\n\n", warnings.join("\n- "))
    }
}

fn truncate_discord(input: &str) -> String {
    if input.chars().count() <= MAX_DISCORD_CONTENT_CHARS {
        return input.to_string();
    }
    let mut out: String = input
        .chars()
        .take(MAX_DISCORD_CONTENT_CHARS.saturating_sub(80))
        .collect();
    out.push_str("\n\n[Gekuerzt; vollstaendige Details im Attachment.]");
    out
}

fn option_i64(
    options: &std::collections::HashMap<String, Value>,
    key: &str,
) -> ServerSyncResult<i64> {
    let value = options
        .get(key)
        .ok_or_else(|| ServerSyncError::bad_request(format!("{key} fehlt")))?;
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    if let Some(value) = value.as_u64().and_then(|value| i64::try_from(value).ok()) {
        return Ok(value);
    }
    Err(ServerSyncError::bad_request(format!(
        "{key} muss eine Zahl sein"
    )))
}

fn option_i64_optional(
    options: &std::collections::HashMap<String, Value>,
    key: &str,
) -> ServerSyncResult<Option<i64>> {
    if !options.contains_key(key) || options.get(key).is_some_and(Value::is_null) {
        return Ok(None);
    }
    option_i64(options, key).map(Some)
}

fn option_string(
    options: &std::collections::HashMap<String, Value>,
    key: &str,
) -> ServerSyncResult<String> {
    let value = options
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if value.is_empty() {
        Err(ServerSyncError::bad_request(format!("{key} fehlt")))
    } else {
        Ok(value)
    }
}

fn option_bool(options: &std::collections::HashMap<String, Value>, key: &str) -> Option<bool> {
    options.get(key).and_then(Value::as_bool)
}

#[derive(Clone)]
pub struct HttpState {
    service: SharedServerSync,
    token: Option<String>,
}

pub fn router(service: SharedServerSync, token: Option<String>) -> Router {
    let token = token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Router::new()
        .route("/serversync/snapshot", post(http_snapshot))
        .route("/serversync/rollback-export", post(http_rollback_export))
        .route("/serversync/diff", post(http_diff))
        .route("/serversync/restore", post(http_restore))
        .route(
            "/serversync/onboarding-preview",
            post(http_onboarding_preview),
        )
        .route(
            "/serversync/serverguide-preview",
            post(http_serverguide_preview),
        )
        .route("/serversync/archive-enable", post(http_archive_enable))
        .route("/serversync/archive-disable", post(http_archive_disable))
        .route(
            "/serversync/regelwerk-publish",
            post(http_regelwerk_publish),
        )
        .route("/serversync/regelwerk-apply", post(http_regelwerk_apply))
        .route("/serversync/support-apply", post(http_support_apply))
        .route("/serversync/faq-apply", post(http_faq_apply))
        .route("/serversync/welcome-preview", post(http_welcome_preview))
        .route("/serversync/welcome-apply", post(http_welcome_apply))
        .route("/serversync/rang-guide-apply", post(http_rang_guide_apply))
        .route("/serversync/router-apply", post(http_router_apply))
        .route("/serversync/lfg-panel-apply", post(http_lfg_panel_apply))
        .route("/serversync/onboarding-apply", post(http_onboarding_apply))
        .route(
            "/serversync/serverguide-apply",
            post(http_serverguide_apply),
        )
        .route("/serversync/apply", post(http_apply))
        .with_state(HttpState { service, token })
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn json_error(error: ServerSyncError) -> Response {
    json_response(
        error.status(),
        json!({
            "ok": false,
            "error": error.to_string(),
        }),
    )
}

fn json_ok(result: Value) -> Response {
    json_response(StatusCode::OK, json!({ "ok": true, "result": result }))
}

fn parse_json_body<T>(body: Bytes) -> ServerSyncResult<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(&body)
        .map_err(|err| ServerSyncError::bad_request(format!("ungueltiges JSON: {err}")))
}

fn authorize(
    state: &HttpState,
    peer: &SocketAddr,
    headers: &HeaderMap,
) -> Result<(), ServerSyncError> {
    if !peer.ip().is_loopback() {
        return Err(ServerSyncError::forbidden("loopback requests only"));
    }
    let Some(expected) = state.token.as_deref().filter(|token| !token.is_empty()) else {
        return Err(ServerSyncError::forbidden(
            "SERVERSYNC_INTERNAL_TOKEN ist nicht konfiguriert",
        ));
    };
    let presented = headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim();
    if presented.is_empty() {
        return Err(ServerSyncError::forbidden(format!("{TOKEN_HEADER} fehlt")));
    }
    if !constant_time_eq(presented, expected) {
        return Err(ServerSyncError::unauthorized(format!(
            "{TOKEN_HEADER} ist ungueltig"
        )));
    }
    Ok(())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let left = Sha256::digest(a.as_bytes());
    let right = Sha256::digest(b.as_bytes());
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

async fn http_snapshot(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.snapshot(None).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_rollback_export(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.rollback_export(None).await {
        Ok(output) => json_ok(json!({
            "rollback_export_id": output.rollback_export_id,
            "snapshot_id": output.snapshot_id,
            "guild_id": output.guild_id,
            "artifact_hash": output.artifact_hash,
            "captured_at": output.captured_at,
            "members": output.members,
            "member_role_edges": output.member_role_edges,
            "filename": output.filename,
            "artifact": output.artifact,
        })),
        Err(error) => json_error(error),
    }
}

async fn http_diff(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.diff(None).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

#[derive(Debug, Default, Deserialize)]
struct RestoreRequest {
    rollback_export_id: Option<i64>,
}

async fn http_restore(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        RestoreRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.restore(body.rollback_export_id, None).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_onboarding_preview(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.onboarding_preview(None).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_serverguide_preview(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.serverguide_preview(None).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_archive_enable(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.archive_enable().await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_archive_disable(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.archive_disable().await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ConfirmRequest {
    #[serde(default)]
    confirm: bool,
}

async fn http_regelwerk_publish(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.regelwerk_publish(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_regelwerk_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.regelwerk_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_support_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.support_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_faq_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.faq_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_welcome_preview(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    match state.service.welcome_preview().await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_welcome_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.welcome_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_rang_guide_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.rang_guide_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_router_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.router_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_lfg_panel_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = if body.is_empty() {
        ConfirmRequest::default()
    } else {
        match parse_json_body(body) {
            Ok(body) => body,
            Err(error) => return json_error(error),
        }
    };
    match state.service.lfg_panel_apply(body.confirm).await {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_onboarding_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = match parse_json_body::<ApplyRequest>(body) {
        Ok(body) => body,
        Err(error) => return json_error(error),
    };
    match state
        .service
        .onboarding_apply(body.preview_id, body.hash, body.confirm, None)
        .await
    {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

async fn http_serverguide_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = match parse_json_body::<ApplyRequest>(body) {
        Ok(body) => body,
        Err(error) => return json_error(error),
    };
    match state
        .service
        .serverguide_apply(body.preview_id, body.hash, body.confirm, None)
        .await
    {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

#[derive(Debug, Deserialize)]
struct ApplyRequest {
    preview_id: i64,
    hash: String,
    #[serde(default)]
    confirm: bool,
}

async fn http_apply(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
    let body = match parse_json_body::<ApplyRequest>(body) {
        Ok(body) => body,
        Err(error) => return json_error(error),
    };
    match state
        .service
        .apply(body.preview_id, body.hash, body.confirm, None)
        .await
    {
        Ok(output) => json_ok(json!(output)),
        Err(error) => json_error(error),
    }
}

#[allow(dead_code)]
fn _assert_diff_serializable(diff: &ServerDiff) -> serde_json::Result<String> {
    serde_json::to_string(diff)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::{Path as AxumPath, Query, State};
    use axum::http::Request;
    use axum::response::IntoResponse;
    use axum::routing::{get, patch, put};
    use dl_server_as_code::{
        BotMessageSpec, CategorySpec, ChannelKind, ChannelSpec, OverwriteKey, RoleSpec, TargetKind,
    };
    use serenity::all::Permissions;
    use tower::ServiceExt;

    use super::*;

    type ApplyCall = (i64, String, bool, Option<u64>);
    type OnboardingApplyCall = (i64, String, bool, Option<u64>);
    type ServerGuideApplyCall = (i64, String, bool, Option<u64>);
    type RestoreCall = (Option<i64>, Option<u64>);
    type ConfirmCall = bool;
    type WelcomeApplyCall = bool;
    type RouterApplyCall = bool;

    #[derive(Clone)]
    struct FakeDiscord {
        base_url: String,
        state: Arc<FakeDiscordState>,
    }

    struct FakeDiscordState {
        bot_user_id: u64,
        messages: Mutex<BTreeMap<u64, Value>>,
        post_responses: Mutex<VecDeque<FakePostResponse>>,
        next_post_id: Mutex<u64>,
        post_calls: Mutex<Vec<u64>>,
        post_bodies: Mutex<Vec<(u64, Value)>>,
        patch_calls: Mutex<Vec<u64>>,
        pin_calls: Mutex<Vec<(u64, u64)>>,
        patched_channels: Mutex<Vec<(u64, Value)>>,
        delete_calls: Mutex<Vec<u64>>,
    }

    enum FakePostResponse {
        Ok(u64),
        Status(StatusCode),
    }

    #[derive(Default)]
    struct MockServerSync {
        apply_calls: Mutex<Vec<ApplyCall>>,
        onboarding_apply_calls: Mutex<Vec<OnboardingApplyCall>>,
        serverguide_apply_calls: Mutex<Vec<ServerGuideApplyCall>>,
        restore_calls: Mutex<Vec<RestoreCall>>,
        archive_calls: Mutex<Vec<bool>>,
        regelwerk_calls: Mutex<Vec<ConfirmCall>>,
        regelwerk_apply_calls: Mutex<Vec<ConfirmCall>>,
        support_apply_calls: Mutex<Vec<ConfirmCall>>,
        faq_apply_calls: Mutex<Vec<ConfirmCall>>,
        welcome_apply_calls: Mutex<Vec<WelcomeApplyCall>>,
        rang_guide_apply_calls: Mutex<Vec<ConfirmCall>>,
        router_apply_calls: Mutex<Vec<RouterApplyCall>>,
        lfg_panel_apply_calls: Mutex<Vec<ConfirmCall>>,
        lfg_panel_apply_override: Mutex<Option<LfgPanelApplyOutput>>,
    }

    #[async_trait]
    impl ServerSyncOps for MockServerSync {
        fn guild_id(&self) -> u64 {
            GUILD_ID
        }

        async fn snapshot(
            &self,
            _requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<SnapshotOutput> {
            Ok(SnapshotOutput {
                snapshot_id: 10,
                guild_id: GUILD_ID,
                captured_at: "2026-07-02T00:00:00.000Z".to_string(),
                categories: 1,
                channels: 2,
                roles: 3,
                overwrites: 4,
            })
        }

        async fn rollback_export(
            &self,
            _requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<RollbackExportOutput> {
            Ok(RollbackExportOutput {
                rollback_export_id: 7,
                snapshot_id: 10,
                guild_id: GUILD_ID,
                artifact_hash: "abc".to_string(),
                captured_at: "2026-07-02T00:00:00.000Z".to_string(),
                members: 2,
                member_role_edges: 3,
                filename: "rollback.json".to_string(),
                artifact_text: "{}".to_string(),
                artifact: json!({"member_role_assignments": [
                    {"member_id": 1_u64, "role_ids": [2_u64, 3_u64]}
                ]}),
            })
        }

        async fn diff(&self, _requested_by_user_id: Option<u64>) -> ServerSyncResult<DiffOutput> {
            Ok(DiffOutput {
                preview_id: 11,
                guild_id: GUILD_ID,
                diff_hash: "hash".to_string(),
                human_summary: "summary".to_string(),
                warnings: vec!["warn".to_string()],
                diff_text: "{\"changes\":[]}".to_string(),
            })
        }

        async fn restore(
            &self,
            rollback_export_id: Option<i64>,
            requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<RestoreOutput> {
            self.restore_calls
                .lock()
                .expect("restore calls")
                .push((rollback_export_id, requested_by_user_id));
            Ok(RestoreOutput {
                rollback_export_id: rollback_export_id.unwrap_or(7),
                preview_id: 13,
                guild_id: GUILD_ID,
                artifact_hash: "artifact-hash".to_string(),
                diff_hash: "restore-hash".to_string(),
                human_summary: "restore summary".to_string(),
                warnings: vec!["restore warn".to_string()],
                diff_text: "{\"restore\":true}".to_string(),
            })
        }

        async fn apply(
            &self,
            preview_id: i64,
            hash: String,
            confirm: bool,
            requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<ApplyOutput> {
            self.apply_calls.lock().expect("apply calls").push((
                preview_id,
                hash,
                confirm,
                requested_by_user_id,
            ));
            Ok(ApplyOutput {
                apply_run_id: 12,
                preview_id,
                dry_run: !confirm,
                applied: usize::from(confirm),
                skipped: usize::from(!confirm),
                failed: 0,
                details_text: "{}".to_string(),
                details: json!({}),
            })
        }

        async fn onboarding_preview(
            &self,
            _requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<OnboardingPreviewOutput> {
            Ok(OnboardingPreviewOutput {
                preview_id: Some(21),
                guild_id: GUILD_ID,
                diff_hash: Some("onboarding-hash".to_string()),
                human_summary: "Onboarding-Diff: 1 Änderung(en).".to_string(),
                blockers: Vec::new(),
                warnings: vec!["onboarding warn".to_string()],
                diff_text: "{\"onboarding\":true}".to_string(),
                desired_config: Some(json!({"enabled": true})),
            })
        }

        async fn onboarding_apply(
            &self,
            preview_id: i64,
            hash: String,
            confirm: bool,
            requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<ApplyOutput> {
            self.onboarding_apply_calls
                .lock()
                .expect("onboarding apply calls")
                .push((preview_id, hash, confirm, requested_by_user_id));
            Ok(ApplyOutput {
                apply_run_id: 22,
                preview_id,
                dry_run: !confirm,
                applied: usize::from(confirm),
                skipped: usize::from(!confirm),
                failed: 0,
                details_text: "{}".to_string(),
                details: json!({}),
            })
        }

        async fn serverguide_preview(
            &self,
            _requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<ServerGuidePreviewOutput> {
            Ok(ServerGuidePreviewOutput {
                preview_id: Some(31),
                guild_id: GUILD_ID,
                diff_hash: Some("serverguide-hash".to_string()),
                human_summary: "Server-Guide-Diff: 1 Änderung(en).".to_string(),
                blockers: Vec::new(),
                warnings: vec!["serverguide warn".to_string()],
                diff_text: "{\"serverguide\":true}".to_string(),
                desired_config: Some(json!({"enabled": true})),
            })
        }

        async fn serverguide_apply(
            &self,
            preview_id: i64,
            hash: String,
            confirm: bool,
            requested_by_user_id: Option<u64>,
        ) -> ServerSyncResult<ApplyOutput> {
            self.serverguide_apply_calls
                .lock()
                .expect("serverguide apply calls")
                .push((preview_id, hash, confirm, requested_by_user_id));
            Ok(ApplyOutput {
                apply_run_id: 32,
                preview_id,
                dry_run: !confirm,
                applied: usize::from(confirm),
                skipped: usize::from(!confirm),
                failed: 0,
                details_text: "{}".to_string(),
                details: json!({}),
            })
        }

        async fn archive_enable(&self) -> ServerSyncResult<ArchiveFlagOutput> {
            self.archive_calls.lock().expect("archive calls").push(true);
            Ok(ArchiveFlagOutput {
                guild_id: GUILD_ID,
                enabled: true,
            })
        }

        async fn archive_disable(&self) -> ServerSyncResult<ArchiveFlagOutput> {
            self.archive_calls
                .lock()
                .expect("archive calls")
                .push(false);
            Ok(ArchiveFlagOutput {
                guild_id: GUILD_ID,
                enabled: false,
            })
        }

        async fn regelwerk_publish(
            &self,
            confirm: bool,
        ) -> ServerSyncResult<RegelwerkPublishOutput> {
            self.regelwerk_calls
                .lock()
                .expect("regelwerk calls")
                .push(confirm);
            Ok(RegelwerkPublishOutput {
                guild_id: GUILD_ID,
                dry_run: !confirm,
                threads_found: 195,
                threads_deleted: if confirm { 195 } else { 0 },
                bot_messages_found: 2,
                bot_messages_deleted: if confirm { 2 } else { 0 },
                bot_message_ids: vec![7001, 7002],
                bot_message_embed_titles: vec!["Hier starten ➜".to_string()],
                stored_message_id: Some(7003),
                posted_message_id: None,
                edited_message_id: confirm.then_some(7003),
            })
        }

        async fn regelwerk_apply(
            &self,
            confirm: bool,
        ) -> ServerSyncResult<regelwerk_publish::RegelwerkPublishOutput> {
            self.regelwerk_apply_calls
                .lock()
                .expect("regelwerk apply calls")
                .push(confirm);
            Ok(mock_regelwerk_output(!confirm))
        }

        async fn support_apply(&self, confirm: bool) -> ServerSyncResult<SupportPublishOutput> {
            self.support_apply_calls
                .lock()
                .expect("support apply calls")
                .push(confirm);
            Ok(mock_support_output(!confirm))
        }

        async fn faq_apply(&self, confirm: bool) -> ServerSyncResult<FaqPublishOutput> {
            self.faq_apply_calls
                .lock()
                .expect("faq apply calls")
                .push(confirm);
            Ok(mock_faq_output(!confirm))
        }

        async fn welcome_preview(&self) -> ServerSyncResult<WelcomePublishOutput> {
            Ok(mock_welcome_output(true))
        }

        async fn welcome_apply(&self, confirm: bool) -> ServerSyncResult<WelcomePublishOutput> {
            self.welcome_apply_calls
                .lock()
                .expect("welcome apply calls")
                .push(confirm);
            Ok(mock_welcome_output(!confirm))
        }

        async fn rang_guide_apply(
            &self,
            confirm: bool,
        ) -> ServerSyncResult<RangGuidePublishOutput> {
            self.rang_guide_apply_calls
                .lock()
                .expect("rang guide apply calls")
                .push(confirm);
            Ok(mock_rang_guide_output(!confirm))
        }

        async fn router_apply(&self, confirm: bool) -> ServerSyncResult<RouterApplyOutput> {
            self.router_apply_calls
                .lock()
                .expect("router apply calls")
                .push(confirm);
            Ok(mock_router_output(!confirm))
        }

        async fn lfg_panel_apply(&self, confirm: bool) -> ServerSyncResult<LfgPanelApplyOutput> {
            self.lfg_panel_apply_calls
                .lock()
                .expect("lfg panel apply calls")
                .push(confirm);
            if let Some(output) = self
                .lfg_panel_apply_override
                .lock()
                .expect("lfg panel apply override")
                .clone()
            {
                return Ok(output);
            }
            Ok(mock_lfg_panel_output(!confirm))
        }
    }

    fn request(path: &str, token: Option<&str>, body: Value) -> Request<Body> {
        request_raw(path, token, body.to_string())
    }

    fn request_raw(path: &str, token: Option<&str>, body: impl Into<String>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header(TOKEN_HEADER, token);
        }
        let mut request = builder.body(Body::from(body.into())).expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4444))));
        request
    }

    async fn response_json(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn spawn_fake_discord(
        bot_user_id: u64,
        messages: Vec<Value>,
        post_responses: Vec<FakePostResponse>,
    ) -> FakeDiscord {
        let state = Arc::new(FakeDiscordState {
            bot_user_id,
            messages: Mutex::new(
                messages
                    .into_iter()
                    .map(|message| {
                        let id = message["id"]
                            .as_str()
                            .expect("message id")
                            .parse::<u64>()
                            .expect("message id u64");
                        (id, message)
                    })
                    .collect(),
            ),
            post_responses: Mutex::new(post_responses.into()),
            next_post_id: Mutex::new(90_000),
            post_calls: Mutex::new(Vec::new()),
            post_bodies: Mutex::new(Vec::new()),
            patch_calls: Mutex::new(Vec::new()),
            pin_calls: Mutex::new(Vec::new()),
            patched_channels: Mutex::new(Vec::new()),
            delete_calls: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route("/api/v10/users/@me", get(fake_discord_current_user))
            .route(
                "/api/v10/channels/{channel_id}/messages",
                get(fake_discord_list_messages).post(fake_discord_post_message),
            )
            .route(
                "/api/v10/channels/{channel_id}/messages/{message_id}",
                get(fake_discord_get_message)
                    .patch(fake_discord_patch_message)
                    .delete(fake_discord_delete_message),
            )
            .route(
                "/api/v10/channels/{channel_id}/pins/{message_id}",
                put(fake_discord_pin_message),
            )
            .route(
                "/api/v10/channels/{channel_id}/threads",
                post(fake_discord_create_thread),
            )
            .route(
                "/api/v10/channels/{channel_id}",
                patch(fake_discord_patch_channel),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake discord bind");
        let addr = listener.local_addr().expect("fake discord addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("fake discord serve");
        });
        FakeDiscord {
            base_url: format!("http://{addr}/api/v10"),
            state,
        }
    }

    async fn fake_discord_current_user(State(state): State<Arc<FakeDiscordState>>) -> Json<Value> {
        Json(json!({"id": state.bot_user_id.to_string()}))
    }

    async fn fake_discord_list_messages(
        State(state): State<Arc<FakeDiscordState>>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Json<Vec<Value>> {
        let before = query
            .get("before")
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(u64::MAX);
        let limit = query
            .get("limit")
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(100);
        let messages = state.messages.lock().expect("messages");
        Json(
            messages
                .iter()
                .rev()
                .filter(|(message_id, _)| **message_id < before)
                .take(limit)
                .map(|(_, message)| message.clone())
                .collect(),
        )
    }

    async fn fake_discord_get_message(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath((_channel_id, message_id)): AxumPath<(u64, u64)>,
    ) -> Response {
        state
            .messages
            .lock()
            .expect("messages")
            .get(&message_id)
            .cloned()
            .map(Json)
            .map(IntoResponse::into_response)
            .unwrap_or_else(|| {
                (StatusCode::NOT_FOUND, Json(json!({"message": "missing"}))).into_response()
            })
    }

    async fn fake_discord_post_message(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath(channel_id): AxumPath<u64>,
        body: Bytes,
    ) -> Response {
        state
            .post_calls
            .lock()
            .expect("post calls")
            .push(channel_id);
        let parsed_body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
        state
            .post_bodies
            .lock()
            .expect("post bodies")
            .push((channel_id, parsed_body.clone()));
        let response = state
            .post_responses
            .lock()
            .expect("post responses")
            .pop_front();
        match response.unwrap_or_else(|| {
            let mut next = state.next_post_id.lock().expect("next post id");
            let id = *next;
            *next += 1;
            FakePostResponse::Ok(id)
        }) {
            FakePostResponse::Ok(message_id) => {
                state.messages.lock().expect("messages").insert(
                    message_id,
                    json!({
                        "id": message_id.to_string(),
                        "author": {"id": state.bot_user_id.to_string()},
                        "content": "",
                        "flags": parsed_body
                            .get("flags")
                            .and_then(Value::as_u64)
                            .unwrap_or(voice_ux_publish::VOICE_UX_COMPONENTS_V2_FLAG),
                        "pinned": false,
                        "components": parsed_body
                            .get("components")
                            .cloned()
                            .unwrap_or_else(|| json!([])),
                        "embeds": [],
                    }),
                );
                Json(json!({"id": message_id.to_string()})).into_response()
            }
            FakePostResponse::Status(status) => {
                (status, Json(json!({"message": "forced failure"}))).into_response()
            }
        }
    }

    async fn fake_discord_create_thread(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath(channel_id): AxumPath<u64>,
        body: Bytes,
    ) -> Response {
        state
            .post_calls
            .lock()
            .expect("post calls")
            .push(channel_id);
        let parsed_body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
        state
            .post_bodies
            .lock()
            .expect("post bodies")
            .push((channel_id, parsed_body));
        let response = state
            .post_responses
            .lock()
            .expect("post responses")
            .pop_front();
        match response.unwrap_or_else(|| {
            let mut next = state.next_post_id.lock().expect("next post id");
            let id = *next;
            *next += 1;
            FakePostResponse::Ok(id)
        }) {
            FakePostResponse::Ok(thread_id) => {
                Json(json!({"id": thread_id.to_string()})).into_response()
            }
            FakePostResponse::Status(status) => {
                (status, Json(json!({"message": "forced failure"}))).into_response()
            }
        }
    }

    async fn fake_discord_patch_message(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath((_channel_id, message_id)): AxumPath<(u64, u64)>,
        _body: Bytes,
    ) -> Response {
        state
            .patch_calls
            .lock()
            .expect("patch calls")
            .push(message_id);
        if state
            .messages
            .lock()
            .expect("messages")
            .contains_key(&message_id)
        {
            Json(json!({"id": message_id.to_string()})).into_response()
        } else {
            (StatusCode::NOT_FOUND, Json(json!({"message": "missing"}))).into_response()
        }
    }

    async fn fake_discord_delete_message(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath((_channel_id, message_id)): AxumPath<(u64, u64)>,
    ) -> Response {
        state
            .delete_calls
            .lock()
            .expect("delete calls")
            .push(message_id);
        let removed = state
            .messages
            .lock()
            .expect("messages")
            .remove(&message_id)
            .is_some();
        if removed {
            StatusCode::NO_CONTENT.into_response()
        } else {
            (StatusCode::NOT_FOUND, Json(json!({"message": "missing"}))).into_response()
        }
    }

    async fn fake_discord_pin_message(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath((channel_id, message_id)): AxumPath<(u64, u64)>,
    ) -> StatusCode {
        state
            .pin_calls
            .lock()
            .expect("pin calls")
            .push((channel_id, message_id));
        if let Some(message) = state
            .messages
            .lock()
            .expect("messages")
            .get_mut(&message_id)
        {
            message["pinned"] = json!(true);
            StatusCode::NO_CONTENT
        } else {
            StatusCode::NOT_FOUND
        }
    }

    async fn fake_discord_patch_channel(
        State(state): State<Arc<FakeDiscordState>>,
        AxumPath(channel_id): AxumPath<u64>,
        body: Bytes,
    ) -> Json<Value> {
        let parsed_body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
        state
            .patched_channels
            .lock()
            .expect("patched channels")
            .push((channel_id, parsed_body));
        Json(json!({"id": channel_id.to_string()}))
    }

    fn fake_discord_message(
        message_id: u64,
        author_id: u64,
        flags: u64,
        components: Vec<Value>,
        embeds: Vec<Value>,
    ) -> Value {
        fake_discord_message_with_content(message_id, author_id, flags, components, embeds, "")
    }

    fn fake_discord_message_with_content(
        message_id: u64,
        author_id: u64,
        flags: u64,
        components: Vec<Value>,
        embeds: Vec<Value>,
        content: &str,
    ) -> Value {
        json!({
            "id": message_id.to_string(),
            "author": {"id": author_id.to_string()},
            "content": content,
            "flags": flags,
            "pinned": false,
            "components": components,
            "embeds": embeds,
        })
    }

    fn write_test_rang_repo(repo_root: &std::path::Path, step2_body: Option<&str>) {
        let texts_path = repo_root.join(rang_guide_publish::RANG_GUIDE_TEXTS_FILE);
        std::fs::create_dir_all(texts_path.parent().expect("texts parent"))
            .expect("mkdir texts parent");
        if let Some(step2_body) = step2_body {
            std::fs::write(
                texts_path,
                format!("[texts]\nstep2_body = \"{step2_body}\"\n"),
            )
            .expect("write texts");
        }
        let banner_path = repo_root.join(format!(
            "{}/{}",
            rang_guide_publish::RANG_GUIDE_BANNER_DIR,
            rang_guide_publish::RANG_GUIDE_HERO_FILENAME
        ));
        std::fs::create_dir_all(banner_path.parent().expect("banner parent"))
            .expect("mkdir banner parent");
        std::fs::write(banner_path, b"banner").expect("write banner");
    }

    fn write_test_regelwerk_repo(repo_root: &std::path::Path) {
        let texts_path = repo_root.join(regelwerk_publish::REGELWERK_TEXTS_FILE);
        std::fs::create_dir_all(texts_path.parent().expect("texts parent"))
            .expect("mkdir texts parent");
        std::fs::write(
            texts_path,
            r#"
[texts]
title = "**📜 Regelwerk · Deutsche Deadlock Community**"
"#,
        )
        .expect("write texts");
        let banner_path = repo_root.join(format!(
            "{}/{}",
            regelwerk_publish::REGELWERK_BANNER_DIR,
            regelwerk_publish::REGELWERK_HERO_FILENAME
        ));
        std::fs::create_dir_all(banner_path.parent().expect("banner parent"))
            .expect("mkdir banner parent");
        std::fs::write(banner_path, b"regelwerk-banner").expect("write banner");
    }

    fn write_test_support_repo(repo_root: &std::path::Path) {
        let texts_path = repo_root.join(support_publish::SUPPORT_TEXTS_FILE);
        std::fs::create_dir_all(texts_path.parent().expect("texts parent"))
            .expect("mkdir texts parent");
        std::fs::write(
            texts_path,
            r#"
[texts]
title = "**💜 Server unterstützen**"
"#,
        )
        .expect("write texts");
        let banner_path = repo_root.join(format!(
            "{}/{}",
            support_publish::SUPPORT_BANNER_DIR,
            support_publish::SUPPORT_HERO_FILENAME
        ));
        std::fs::create_dir_all(banner_path.parent().expect("banner parent"))
            .expect("mkdir banner parent");
        std::fs::write(banner_path, b"support-banner").expect("write banner");
    }

    fn write_test_faq_repo(repo_root: &std::path::Path) {
        let texts_path = repo_root.join(faq_publish::FAQ_TEXTS_FILE);
        std::fs::create_dir_all(texts_path.parent().expect("texts parent"))
            .expect("mkdir texts parent");
        std::fs::write(
            texts_path,
            r#"
[texts]
title = "**❓ Server-FAQ · Deutsche Deadlock Community**"
"#,
        )
        .expect("write texts");
        let banner_path = repo_root.join(format!(
            "{}/{}",
            faq_publish::FAQ_BANNER_DIR,
            faq_publish::FAQ_HERO_FILENAME
        ));
        std::fs::create_dir_all(banner_path.parent().expect("banner parent"))
            .expect("mkdir banner parent");
        std::fs::write(banner_path, b"faq-banner").expect("write banner");
    }

    fn write_test_voice_ux_repo(repo_root: &std::path::Path, bytes: &[u8]) {
        for filename in [
            voice_ux_publish::VOICE_UX_GUIDE_BANNER_FILENAME,
            voice_ux_publish::VOICE_UX_LFG_BANNER_FILENAME,
            voice_ux_publish::VOICE_UX_SPAWN_BANNER_FILENAME,
            voice_ux_publish::VOICE_UX_MANAGE_BANNER_FILENAME,
        ] {
            let banner_path = repo_root.join(format!(
                "{}/{}",
                voice_ux_publish::VOICE_UX_BANNER_DIR,
                filename
            ));
            std::fs::create_dir_all(banner_path.parent().expect("banner parent"))
                .expect("mkdir banner parent");
            std::fs::write(banner_path, bytes).expect("write voice ux banner");
        }
    }

    async fn set_serversync_kv(pool: &PgPool, key: &str, value: &str) {
        dl_central_db::kv::set(pool, SERVERSYNC_KV_NS, key, value)
            .await
            .expect("set serversync kv");
    }

    async fn get_serversync_kv(pool: &PgPool, key: &str) -> Option<String> {
        dl_central_db::kv::get(pool, SERVERSYNC_KV_NS, key)
            .await
            .expect("get serversync kv")
    }

    fn mock_welcome_output(dry_run: bool) -> WelcomePublishOutput {
        WelcomePublishOutput {
            guild_id: GUILD_ID,
            channel_id: 9001,
            channel_name: "🧭willkommen".to_string(),
            dry_run,
            payload_format: welcome_publish::WELCOME_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: Some(welcome_publish::WELCOME_PAYLOAD_FORMAT.to_string()),
            repost_required: false,
            warnings: vec!["banner fehlt".to_string()],
            team_roles: vec![welcome_publish::WelcomeTeamRoleOutput {
                key: "community-moderator".to_string(),
                aliases: vec![
                    "Community Moderator".to_string(),
                    "Community Mod".to_string(),
                ],
                matched_role_id: Some(42),
                matched_role_name: Some("Community Moderator".to_string()),
                member_ids: vec![100],
                member_mentions: vec!["<@100>".to_string()],
            }],
            bot_team: Some(welcome_publish::WelcomeTeamBotOutput {
                user_id: 999,
                mention: "<@999>".to_string(),
                group_title: "🤖 Server-Management".to_string(),
                description:
                    "unser Bot: verwaltet Rollen, Voice-Lanes, Onboarding, Coaching und diesen Hub."
                        .to_string(),
            }),
            sections: vec![welcome_publish::WelcomeSectionOutput {
                section_id: "hero".to_string(),
                message_index: 0,
                message_key: "welcome:hero".to_string(),
                marker: welcome_publish::welcome_marker("hero"),
                action: if dry_run { "planned_edit" } else { "edited" }.to_string(),
                stored_message_id: Some(7001),
                message_id: Some(7001),
                banner: Some(welcome_publish::WelcomeBannerOutput {
                    filename: "hero.png".to_string(),
                    relative_path: "assets/welcome-banners/hero.png".to_string(),
                    present: false,
                }),
                payload: welcome_publish::WelcomeMessagePayload {
                    flags: welcome_publish::WELCOME_COMPONENTS_V2_FLAG,
                    allowed_mentions: welcome_publish::WelcomeAllowedMentions { parse: Vec::new() },
                    components: vec![json!({
                        "type": 17,
                        "accent_color": welcome_publish::WELCOME_ACCENT_GOLD,
                        "components": [{
                            "type": 10,
                            "content": "Platzhalter",
                        }],
                    })],
                    attachments: Vec::new(),
                },
            }],
            stored_message_ids: BTreeMap::from([("hero".to_string(), vec![7001])]),
            posted_message_ids: BTreeMap::new(),
            edited_message_ids: if dry_run {
                BTreeMap::new()
            } else {
                BTreeMap::from([("hero".to_string(), vec![7001])])
            },
        }
    }

    fn mock_rang_guide_output(dry_run: bool) -> RangGuidePublishOutput {
        let payload = rang_guide_publish::RangGuideMessagePayload {
            flags: rang_guide_publish::RANG_GUIDE_COMPONENTS_V2_FLAG,
            allowed_mentions: rang_guide_publish::RangGuideAllowedMentions { parse: Vec::new() },
            components: vec![
                json!({
                    "type": 17,
                    "accent_color": rang_guide_publish::RANG_GUIDE_ACCENT_GOLD,
                    "components": [
                        {
                            "type": 12,
                            "items": [{"media": {"url": "attachment://rang-guide-hero.png"}}],
                        },
                        {
                            "type": 10,
                            "content": rang_guide_publish::RANG_GUIDE_HERO_INTRO,
                        },
                    ],
                }),
                json!({
                    "type": 17,
                    "accent_color": rang_guide_publish::RANG_GUIDE_ACCENT_GOLD,
                    "components": [
                        {"type": 10, "content": rang_guide_publish::RANG_GUIDE_STEP1_BODY},
                        {
                            "type": 1,
                            "components": [{
                                "type": 2,
                                "style": 1,
                                "label": rang_guide_publish::RANG_GUIDE_STEAM_OPEN_BUTTON_LABEL,
                                "custom_id": rang_guide_publish::STEAM_LINK_OPEN_CUSTOM_ID,
                            }],
                        },
                    ],
                }),
            ],
            attachments: vec![rang_guide_publish::RangGuidePayloadAttachment {
                id: 0,
                filename: rang_guide_publish::RANG_GUIDE_HERO_FILENAME.to_string(),
                relative_path: format!(
                    "{}/{}",
                    rang_guide_publish::RANG_GUIDE_BANNER_DIR,
                    rang_guide_publish::RANG_GUIDE_HERO_FILENAME
                ),
            }],
        };
        RangGuidePublishOutput {
            guild_id: GUILD_ID,
            channel_id: rang_guide_publish::RANG_GUIDE_CHANNEL_ID,
            channel_name: rang_guide_publish::RANG_GUIDE_CHANNEL_NAME.to_string(),
            dry_run,
            payload_format: rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: Some(rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT.to_string()),
            payload_hash: "rang-guide-hash".to_string(),
            stored_payload_hash: Some("rang-guide-hash".to_string()),
            repost_required: false,
            warnings: Vec::new(),
            messages: vec![rang_guide_publish::RangGuideMessageOutput {
                message_index: 0,
                message_key: "rang-guide".to_string(),
                action: if dry_run { "planned_edit" } else { "no_op" }.to_string(),
                stored_message_id: Some(8201),
                message_id: Some(8201),
                banner: Some(rang_guide_publish::RangGuideBannerOutput {
                    filename: rang_guide_publish::RANG_GUIDE_HERO_FILENAME.to_string(),
                    relative_path: format!(
                        "{}/{}",
                        rang_guide_publish::RANG_GUIDE_BANNER_DIR,
                        rang_guide_publish::RANG_GUIDE_HERO_FILENAME
                    ),
                    present: true,
                }),
                payload,
            }],
            stored_message_ids: vec![8201],
            posted_message_ids: Vec::new(),
            edited_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
            legacy_cleanup_candidates: vec![rang_guide_publish::RangGuideLegacyCleanupCandidate {
                message_id: 8100,
                custom_ids: vec![rang_guide_publish::STEAM_LINK_OPEN_CUSTOM_ID.to_string()],
            }],
            deleted_legacy_message_ids: if dry_run { Vec::new() } else { vec![8100] },
        }
    }

    fn mock_regelwerk_output(dry_run: bool) -> regelwerk_publish::RegelwerkPublishOutput {
        let payload = regelwerk_publish::RegelwerkMessagePayload {
            flags: regelwerk_publish::REGELWERK_COMPONENTS_V2_FLAG,
            allowed_mentions: regelwerk_publish::RegelwerkAllowedMentions { parse: Vec::new() },
            components: vec![
                json!({
                    "type": 12,
                    "id": regelwerk_publish::REGELWERK_COMPONENT_ID_HERO_MEDIA,
                    "items": [{"media": {"url": "attachment://regelwerk-hero.png"}}],
                }),
                json!({
                    "type": 17,
                    "id": regelwerk_publish::REGELWERK_COMPONENT_ID_MAIN_CONTAINER,
                    "accent_color": regelwerk_publish::REGELWERK_ACCENT_GOLD,
                    "components": [
                        {
                            "type": 10,
                            "content": format!(
                                "{}\n{}",
                                regelwerk_publish::REGELWERK_MAIN_TITLE,
                                regelwerk_publish::REGELWERK_MAIN_BODY
                            ),
                        },
                        {
                            "type": 1,
                            "components": [{
                                "type": 2,
                                "style": 2,
                                "label": regelwerk_publish::REGELWERK_BUTTON_VERHALTEN_LABEL,
                                "custom_id": regelwerk_publish::REGELWERK_VERHALTEN_CUSTOM_ID,
                            }],
                        },
                    ],
                }),
            ],
            attachments: vec![regelwerk_publish::RegelwerkPayloadAttachment {
                id: 0,
                filename: regelwerk_publish::REGELWERK_HERO_FILENAME.to_string(),
                relative_path: format!(
                    "{}/{}",
                    regelwerk_publish::REGELWERK_BANNER_DIR,
                    regelwerk_publish::REGELWERK_HERO_FILENAME
                ),
            }],
        };
        regelwerk_publish::RegelwerkPublishOutput {
            guild_id: GUILD_ID,
            channel_id: regelwerk_publish::REGELWERK_CHANNEL_ID,
            channel_name: regelwerk_publish::REGELWERK_CHANNEL_NAME.to_string(),
            dry_run,
            payload_format: regelwerk_publish::REGELWERK_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: Some(regelwerk_publish::REGELWERK_PAYLOAD_FORMAT.to_string()),
            payload_hash: "regelwerk-hash".to_string(),
            stored_payload_hash: Some("regelwerk-hash".to_string()),
            repost_required: false,
            warnings: Vec::new(),
            messages: vec![regelwerk_publish::RegelwerkMessageOutput {
                message_index: 0,
                message_key: "regelwerk".to_string(),
                action: if dry_run { "planned_edit" } else { "no_op" }.to_string(),
                stored_message_id: Some(8301),
                message_id: Some(8301),
                banner: Some(regelwerk_publish::RegelwerkBannerOutput {
                    filename: regelwerk_publish::REGELWERK_HERO_FILENAME.to_string(),
                    relative_path: format!(
                        "{}/{}",
                        regelwerk_publish::REGELWERK_BANNER_DIR,
                        regelwerk_publish::REGELWERK_HERO_FILENAME
                    ),
                    present: true,
                }),
                payload,
            }],
            stored_message_ids: vec![8301],
            posted_message_ids: Vec::new(),
            edited_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
            legacy_cleanup_candidate: Some(regelwerk_publish::RegelwerkLegacyCleanupCandidate {
                message_id: regelwerk_publish::REGELWERK_OLD_MESSAGE_ID,
                content_prefix: regelwerk_publish::REGELWERK_LEGACY_CONTENT_PREFIX.to_string(),
            }),
            deleted_legacy_message_ids: if dry_run {
                Vec::new()
            } else {
                vec![regelwerk_publish::REGELWERK_OLD_MESSAGE_ID]
            },
        }
    }

    fn mock_support_output(dry_run: bool) -> SupportPublishOutput {
        let payload = support_publish::SupportMessagePayload {
            flags: support_publish::SUPPORT_COMPONENTS_V2_FLAG,
            allowed_mentions: support_publish::SupportAllowedMentions { parse: Vec::new() },
            components: vec![
                json!({
                    "type": 12,
                    "id": support_publish::SUPPORT_COMPONENT_ID_HERO_MEDIA,
                    "items": [{"media": {"url": "attachment://support-hero.png"}}],
                }),
                json!({
                    "type": 17,
                    "id": support_publish::SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
                    "accent_color": support_publish::SUPPORT_ACCENT_GOLD,
                    "components": [
                        {
                            "type": 10,
                            "content": support_publish::SUPPORT_TITLE,
                        },
                        {
                            "type": 1,
                            "components": [{
                                "type": 2,
                                "style": 5,
                                "label": support_publish::SUPPORT_DONATE_BUTTON_LABEL,
                                "url": support_publish::SUPPORT_DONATE_URL_DEFAULT,
                            }],
                        },
                    ],
                }),
            ],
            attachments: vec![support_publish::SupportPayloadAttachment {
                id: 0,
                filename: support_publish::SUPPORT_HERO_FILENAME.to_string(),
                relative_path: format!(
                    "{}/{}",
                    support_publish::SUPPORT_BANNER_DIR,
                    support_publish::SUPPORT_HERO_FILENAME
                ),
            }],
        };
        SupportPublishOutput {
            guild_id: GUILD_ID,
            channel_id: support_publish::SUPPORT_CHANNEL_ID,
            channel_name: support_publish::SUPPORT_CHANNEL_NAME.to_string(),
            dry_run,
            payload_format: support_publish::SUPPORT_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: Some(support_publish::SUPPORT_PAYLOAD_FORMAT.to_string()),
            payload_hash: "support-hash".to_string(),
            stored_payload_hash: Some("support-hash".to_string()),
            repost_required: false,
            warnings: Vec::new(),
            messages: vec![support_publish::SupportMessageOutput {
                message_index: 0,
                message_key: "support".to_string(),
                action: if dry_run { "planned_edit" } else { "no_op" }.to_string(),
                stored_message_id: Some(8401),
                message_id: Some(8401),
                banner: Some(support_publish::SupportBannerOutput {
                    filename: support_publish::SUPPORT_HERO_FILENAME.to_string(),
                    relative_path: format!(
                        "{}/{}",
                        support_publish::SUPPORT_BANNER_DIR,
                        support_publish::SUPPORT_HERO_FILENAME
                    ),
                    present: true,
                }),
                payload,
            }],
            stored_message_ids: vec![8401],
            posted_message_ids: Vec::new(),
            edited_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
        }
    }

    fn mock_faq_output(dry_run: bool) -> FaqPublishOutput {
        let payload = faq_publish::FaqMessagePayload {
            flags: faq_publish::FAQ_COMPONENTS_V2_FLAG,
            allowed_mentions: faq_publish::FaqAllowedMentions { parse: Vec::new() },
            components: vec![
                json!({
                    "type": 12,
                    "id": faq_publish::FAQ_COMPONENT_ID_HERO_MEDIA,
                    "items": [{"media": {"url": "attachment://faq-hero.png"}}],
                }),
                json!({
                    "type": 17,
                    "id": faq_publish::FAQ_COMPONENT_ID_MAIN_CONTAINER,
                    "accent_color": faq_publish::FAQ_ACCENT_GOLD,
                    "components": [
                        {
                            "type": 10,
                            "content": format!(
                                "{}\n{}",
                                faq_publish::FAQ_MAIN_TITLE,
                                faq_publish::FAQ_MAIN_BODY
                            ),
                        },
                        {
                            "type": 1,
                            "id": faq_publish::FAQ_COMPONENT_ID_ACTION_ROW,
                            "components": [{
                                "type": 3,
                                "id": faq_publish::FAQ_COMPONENT_ID_SELECT,
                                "custom_id": faq_publish::FAQ_SELECT_CUSTOM_ID,
                                "placeholder": faq_publish::FAQ_SELECT_PLACEHOLDER,
                                "min_values": 1,
                                "max_values": 1,
                                "options": faq_publish::FAQ_DEFAULT_ENTRIES.iter().map(|entry| json!({
                                    "label": entry.label,
                                    "value": entry.value,
                                    "emoji": {"name": entry.emoji},
                                })).collect::<Vec<_>>(),
                            }],
                        },
                    ],
                }),
            ],
            attachments: vec![faq_publish::FaqPayloadAttachment {
                id: 0,
                filename: faq_publish::FAQ_HERO_FILENAME.to_string(),
                relative_path: format!(
                    "{}/{}",
                    faq_publish::FAQ_BANNER_DIR,
                    faq_publish::FAQ_HERO_FILENAME
                ),
            }],
        };
        FaqPublishOutput {
            guild_id: GUILD_ID,
            channel_id: faq_publish::FAQ_CHANNEL_ID,
            channel_name: faq_publish::FAQ_CHANNEL_NAME.to_string(),
            dry_run,
            payload_format: faq_publish::FAQ_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: Some(faq_publish::FAQ_PAYLOAD_FORMAT.to_string()),
            payload_hash: "faq-hash".to_string(),
            stored_payload_hash: Some("faq-hash".to_string()),
            repost_required: false,
            warnings: Vec::new(),
            messages: vec![faq_publish::FaqMessageOutput {
                message_index: 0,
                message_key: "faq".to_string(),
                action: if dry_run { "planned_edit" } else { "no_op" }.to_string(),
                stored_message_id: Some(8501),
                message_id: Some(8501),
                banner: Some(faq_publish::FaqBannerOutput {
                    filename: faq_publish::FAQ_HERO_FILENAME.to_string(),
                    relative_path: format!(
                        "{}/{}",
                        faq_publish::FAQ_BANNER_DIR,
                        faq_publish::FAQ_HERO_FILENAME
                    ),
                    present: true,
                }),
                payload,
            }],
            stored_message_ids: vec![8501],
            posted_message_ids: Vec::new(),
            edited_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
        }
    }

    fn mock_router_output(dry_run: bool) -> RouterApplyOutput {
        let repo = tempfile::tempdir().expect("voice ux repo");
        write_test_voice_ux_repo(repo.path(), b"voice-ux-banner");
        let ids = BTreeMap::from([
            (voice_ux_publish::VOICE_UX_CHANNEL_ID, vec![8001, 8002]),
            (
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                vec![8011, 8012],
            ),
        ]);
        let formats = BTreeMap::from([
            (
                voice_ux_publish::VOICE_UX_CHANNEL_ID,
                Some(voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT.to_string()),
            ),
            (
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                Some(voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT.to_string()),
            ),
        ]);
        let mut output = voice_ux_publish::build_voice_ux_publish_output(
            repo.path(),
            &ids,
            &formats,
            &BTreeMap::new(),
            Some(8021),
            Some("old-hash"),
            dry_run,
        )
        .expect("voice ux mock output");
        if !dry_run {
            for target in &mut output.targets {
                target.edited_message_ids = target.stored_message_ids.clone();
                target.repost_required = false;
                for message in &mut target.messages {
                    message.action = "edited".to_string();
                    message.message_id = message.stored_message_id;
                }
            }
            output.forum_post.action = "edited".to_string();
            output.forum_post.thread_id = output.forum_post.stored_thread_id;
            output.dry_run = false;
        }
        output
    }

    fn mock_lfg_panel_output(dry_run: bool) -> LfgPanelApplyOutput {
        LfgPanelApplyOutput {
            guild_id: GUILD_ID,
            channel_id: Some(8101),
            dry_run,
            payload_format: dl_voice::lfg_panel::LFG_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: Some(dl_voice::lfg_panel::LFG_PAYLOAD_FORMAT.to_string()),
            stored_message_id: Some(8102),
            action: if dry_run { "planned_edit" } else { "edited" }.to_string(),
            message_id: Some(8102),
            blocked_reason: None,
            warnings: Vec::new(),
            payload: json!({
                "flags": dl_voice::lfg_panel::LFG_COMPONENTS_V2_FLAG,
                "allowed_mentions": {"parse": []},
                "components": [{
                    "type": 17,
                    "accent_color": dl_voice::lfg_panel::LFG_ACCENT_GOLD,
                    "components": [
                        {
                            "type": 12,
                            "items": [{"media": {"url": "attachment://router-hero.png"}}],
                        },
                        {
                            "type": 10,
                            "content": dl_voice::lfg_panel::LFG_PANEL_BODY,
                        },
                        {
                            "type": 1,
                            "components": [{
                                "type": 2,
                                "style": 1,
                                "label": dl_voice::lfg_panel::LFG_PANEL_BUTTON,
                                "custom_id": dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID,
                            }],
                        },
                    ],
                }],
                "attachments": [
                    {"id": 0, "filename": dl_voice::lfg_panel::LFG_PANEL_BANNER_FILENAME},
                ],
            }),
        }
    }

    fn v2_artifact_value() -> Value {
        serde_json::to_value(RollbackArtifact {
            version: ROLLBACK_VERSION.to_string(),
            guild_id: GUILD_ID,
            created_at: "2026-07-02T00:00:00.000Z".to_string(),
            snapshot_id: 10,
            structure_snapshot: RollbackGuildModel::from_model(&GuildModel::new(GUILD_ID)),
            dynamic_namespaces: Vec::new(),
            documented_exceptions: Vec::new(),
            member_role_assignments: vec![MemberRoleAssignment {
                member_id: 1,
                role_ids: vec![2, 3],
            }],
            native_onboarding_config: json!({}),
        })
        .expect("artifact value")
    }

    fn rollback_roundtrip_model() -> GuildModel {
        let mut model = GuildModel::new(GUILD_ID);
        model.categories.insert(
            100,
            CategorySpec {
                guild_id: GUILD_ID,
                category_id: 100,
                name: "🗨️Chat".to_string(),
                position: 1,
            },
        );
        model.channels.insert(
            200,
            ChannelSpec {
                guild_id: GUILD_ID,
                channel_id: 200,
                name: "⚖️hier-starten-regelwerk".to_string(),
                kind: ChannelKind::Text,
                topic: Some("Regeln".to_string()),
                position: 1,
                parent_category_id: Some(100),
                nsfw: false,
                bitrate: None,
                user_limit: None,
                rate_limit_per_user: None,
                default_auto_archive_duration: None,
                status: None,
            },
        );
        model.channels.insert(
            201,
            ChannelSpec {
                guild_id: GUILD_ID,
                channel_id: 201,
                name: "Voice".to_string(),
                kind: ChannelKind::Voice,
                topic: None,
                position: 2,
                parent_category_id: Some(100),
                nsfw: false,
                bitrate: Some(64_000),
                user_limit: Some(5),
                rate_limit_per_user: None,
                default_auto_archive_duration: None,
                status: Some("offen".to_string()),
            },
        );
        model.roles.insert(
            GUILD_ID,
            RoleSpec {
                guild_id: GUILD_ID,
                role_id: GUILD_ID,
                name: "@everyone".to_string(),
                color: 0,
                hoist: false,
                mentionable: false,
                managed: false,
                permissions_bitmask: 0,
                position: 0,
            },
        );
        model.roles.insert(
            300,
            RoleSpec {
                guild_id: GUILD_ID,
                role_id: 300,
                name: "VIP".to_string(),
                color: 0xff00ff,
                hoist: true,
                mentionable: true,
                managed: false,
                permissions_bitmask: 42,
                position: 3,
            },
        );
        for overwrite in [
            PermissionOverwriteSpec {
                guild_id: GUILD_ID,
                key: OverwriteKey {
                    channel_id: 200,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                },
                allow_bits: 1,
                deny_bits: 2,
            },
            PermissionOverwriteSpec {
                guild_id: GUILD_ID,
                key: OverwriteKey {
                    channel_id: 200,
                    target_kind: TargetKind::Member,
                    target_id: 400,
                },
                allow_bits: 4,
                deny_bits: 8,
            },
            PermissionOverwriteSpec {
                guild_id: GUILD_ID,
                key: OverwriteKey {
                    channel_id: 201,
                    target_kind: TargetKind::Role,
                    target_id: 300,
                },
                allow_bits: 16,
                deny_bits: 32,
            },
        ] {
            model.overwrites.insert(overwrite.key.clone(), overwrite);
        }
        model.bot_messages.insert(
            (200, "rules-panel".to_string()),
            BotMessageSpec {
                guild_id: GUILD_ID,
                channel_id: 200,
                message_key: "rules-panel".to_string(),
                message_kind: "panel".to_string(),
                message_id: Some(500),
                content_hash: Some("hash".to_string()),
            },
        );
        model
    }

    fn onboarding_role(id: u64, name: &str, mentionable: bool) -> RoleSpec {
        RoleSpec {
            guild_id: GUILD_ID,
            role_id: id,
            name: name.to_string(),
            color: 0,
            hoist: false,
            mentionable,
            managed: false,
            permissions_bitmask: 0,
            position: 1,
        }
    }

    fn onboarding_channel(id: u64, name: &str) -> ChannelSpec {
        ChannelSpec {
            guild_id: GUILD_ID,
            channel_id: id,
            name: name.to_string(),
            kind: ChannelKind::Text,
            topic: None,
            position: 1,
            parent_category_id: None,
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            default_auto_archive_duration: None,
            status: None,
        }
    }

    fn onboarding_model() -> GuildModel {
        let mut model = GuildModel::new(GUILD_ID);
        model.roles.insert(
            GUILD_ID,
            RoleSpec {
                guild_id: GUILD_ID,
                role_id: GUILD_ID,
                name: "@everyone".to_string(),
                color: 0,
                hoist: false,
                mentionable: false,
                managed: false,
                permissions_bitmask: (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES)
                    .bits(),
                position: 0,
            },
        );
        for (id, name, mentionable) in [
            (5001, "Invite-Gast", false),
            (5002, "Frischling", false),
            (5003, "Patchnotes Ping Rolle", true),
            (5004, "Spieler-Suche Ping Rolle", true),
            (5005, "Events & Turniere Ping Rolle", true),
            (5006, "Custom Games Ping Rolle", true),
            (5007, "Streams", true),
            (5008, "Rang-Verknüpfung", false),
            (5009, "Server-Tour", false),
        ] {
            model
                .roles
                .insert(id, onboarding_role(id, name, mentionable));
        }
        for id in 5100..5112 {
            model.roles.insert(
                id,
                onboarding_role(id, &format!("Rank {} (unverifiziert)", id - 5099), false),
            );
        }
        for (id, name) in [
            (6001, "allgemein"),
            (6002, "frag-die-community"),
            (6003, "🎯mitspieler-suche"),
            (6004, "🎲off-topic"),
            (6005, "rank-ups"),
            (6006, "patchnotes"),
            (6007, "deadlock-rang"),
            (6008, "📼gameplay-clips"),
            (6009, "server-support"),
        ] {
            model.channels.insert(id, onboarding_channel(id, name));
        }
        for channel_id in [6006, 6007] {
            let overwrite = PermissionOverwriteSpec {
                guild_id: GUILD_ID,
                key: OverwriteKey {
                    channel_id,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                },
                allow_bits: Permissions::VIEW_CHANNEL.bits(),
                deny_bits: Permissions::SEND_MESSAGES.bits(),
            };
            model.overwrites.insert(overwrite.key.clone(), overwrite);
        }
        model
    }

    #[test]
    fn regelwerk_text_ersetzt_channelnamen_durch_mentions() {
        let text = build_regelwerk_text(&onboarding_model()).expect("regelwerk text");

        assert!(text.contains("- <#6007> — Steam verknüpfen, Rang eintragen"));
        assert!(text.contains(
            "- <#1459628609705738539> — wenn irgendwas nicht funktioniert (Ticket aufmachen)"
        ));
        assert!(text.contains("- <#1491953161747955853> — Fragen über Server, Bots und Concierge"));
        assert!(text.contains("- <#6002> — Community-Fragen und Deadlock-Invite"));
        assert!(text.contains("Probleme? @Moderator oder @Owner pingen"));
        assert!(!text.contains("#deadlock-rang"));
        assert!(!text.contains("<@&"));
    }

    #[test]
    fn channel_resolver_meldet_kanal_duplikat_statt_ersten_treffer() {
        let mut model = onboarding_model();
        model
            .channels
            .insert(6010, onboarding_channel(6010, "💬frag-die-community"));

        let err = require_channel_mention(&model, "frag-die-community")
            .expect_err("duplicate channel names must fail");

        assert!(err.to_string().contains("mehrdeutig"));
        assert!(err.to_string().contains("6002:frag-die-community"));
        assert!(err.to_string().contains("6010:💬frag-die-community"));
    }

    fn serverguide_model() -> GuildModel {
        let mut model = onboarding_model();
        model.overwrites.retain(|key, _| key.channel_id != 6007);
        model.channels.insert(
            RULES_CHANNEL_ID,
            onboarding_channel(RULES_CHANNEL_ID, "regelwerk"),
        );
        model.channels.insert(
            SUPPORT_TICKET_CHANNEL_ID,
            onboarding_channel(SUPPORT_TICKET_CHANNEL_ID, "ticket-eröffnen"),
        );
        model
    }

    fn rank_live_onboarding_config() -> Value {
        let rank_options = (0..12)
            .map(|idx| {
                let mut option = json!({
                    "id": format!("rank-option-{idx}"),
                    "title": format!("Rank {idx}"),
                    "role_ids": [format!("{}", 5100 + idx)],
                    "channel_ids": [],
                });
                if idx == 0 {
                    option["emoji"] = json!({"id": "9000", "name": "ranked", "animated": true});
                }
                option
            })
            .collect::<Vec<_>>();
        json!({
            "enabled": false,
            "mode": 1,
            // 9 gueltige Modell-IDs + eine stale ID (Kanal existiert nicht mehr)
            "default_channel_ids": ["6001","6002","6003","6004","6005","6006","6007","6008","6009","1"],
            "prompts": [
                {
                    "id": "old-meta",
                    "type": 0,
                    "title": "Alter Prompt",
                    "options": [{"id": "old-meta-opt", "title": "Alt", "role_ids": [], "channel_ids": []}],
                    "single_select": true,
                    "required": false,
                    "in_onboarding": true
                },
                {
                    "id": "rank-prompt",
                    "type": 0,
                    "title": "Dein Rang",
                    "options": rank_options,
                    "single_select": true,
                    "required": false,
                    "in_onboarding": true
                }
            ]
        })
    }

    fn live_serverguide_config_with_action_type(action_type: i64) -> Value {
        json!({
            "enabled": false,
            "welcome_message": {
                "author_ids": ["1"],
                "message": "alt"
            },
            "new_member_actions": [
                {
                    "channel_id": "6002",
                    "action_type": action_type,
                    "title": "Alt",
                    "description": ""
                }
            ],
            "resource_channels": []
        })
    }

    #[test]
    fn serverguide_builder_baut_sollconfig_mit_dokumentiertem_action_type() {
        let model = serverguide_model();
        let built = build_server_guide_config(&live_serverguide_config_with_action_type(0), &model);
        let config = built.config.expect("serverguide config");

        assert!(built.blockers.is_empty(), "blockers: {:?}", built.blockers);
        assert!(config.enabled);
        assert_eq!(
            config.welcome_message.author_ids,
            vec!["662995601738170389".to_string()]
        );
        assert_eq!(
            config.welcome_message.message,
            "Schön, dass du da bist! Schau dich in Ruhe um — und wenn du Deadlock noch nicht hast, fragst du in #frag-die-community nach einem Invite."
        );
        assert_eq!(config.new_member_actions.len(), 3);
        assert_eq!(config.new_member_actions[0].title, "Sag Hallo");
        assert_eq!(config.new_member_actions[0].channel_id, "6002");
        assert_eq!(
            config.new_member_actions[0].action_type,
            SERVER_GUIDE_ACTION_TYPE_CHAT
        );
        assert_eq!(
            config.new_member_actions[1].title,
            "Steam verknüpfen & Rang eintragen"
        );
        assert_eq!(config.new_member_actions[1].channel_id, "6007");
        assert_eq!(config.new_member_actions[2].title, "Such dir Mitspieler");
        assert_eq!(config.new_member_actions[2].channel_id, "6003");
        assert_eq!(
            config.new_member_actions[2].action_type,
            SERVER_GUIDE_ACTION_TYPE_VIEW
        );
        assert_eq!(config.resource_channels[0].title, "Regelwerk");
        assert_eq!(
            config.resource_channels[0].channel_id,
            RULES_CHANNEL_ID.to_string()
        );
        assert_eq!(config.resource_channels[1].title, "Patchnotes");
        assert_eq!(config.resource_channels[2].title, "Hilfe & Support");
        assert_eq!(
            config.resource_channels[2].channel_id,
            SUPPORT_TICKET_CHANNEL_ID.to_string()
        );
    }

    #[test]
    fn serverguide_builder_blockt_nur_bei_nicht_sendbarem_chat() {
        let without_live_actions = build_server_guide_config(
            &json!({"enabled": false, "new_member_actions": []}),
            &serverguide_model(),
        );
        assert!(
            without_live_actions.config.is_some(),
            "blockers: {:?}",
            without_live_actions.blockers
        );

        // deadlock-rang (6007) ist eine VIEW-Action: read-only darf NICHT blocken.
        let mut rang_read_only = serverguide_model();
        let rang_overwrite = PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: 6007,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            },
            allow_bits: Permissions::VIEW_CHANNEL.bits(),
            deny_bits: Permissions::SEND_MESSAGES.bits(),
        };
        rang_read_only
            .overwrites
            .insert(rang_overwrite.key.clone(), rang_overwrite);
        let view_ok = build_server_guide_config(
            &live_serverguide_config_with_action_type(0),
            &rang_read_only,
        );
        assert!(view_ok.config.is_some(), "blockers: {:?}", view_ok.blockers);

        // mitspieler-suche (6003) ist eine VIEW-Action: Forum mit Post-Deny darf NICHT blocken.
        let mut lfg_forum_read_only = serverguide_model();
        lfg_forum_read_only
            .channels
            .get_mut(&6003)
            .expect("lfg channel")
            .kind = ChannelKind::Forum;
        let overwrite = PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: 6003,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            },
            allow_bits: Permissions::VIEW_CHANNEL.bits(),
            deny_bits: Permissions::SEND_MESSAGES.bits(),
        };
        lfg_forum_read_only
            .overwrites
            .insert(overwrite.key.clone(), overwrite);
        let lfg_view_ok = build_server_guide_config(
            &live_serverguide_config_with_action_type(0),
            &lfg_forum_read_only,
        );
        assert!(
            lfg_view_ok.config.is_some(),
            "blockers: {:?}",
            lfg_view_ok.blockers
        );

        // Eine CHAT-Action auf einem read-only-Kanal blockt weiter.
        let mut read_only_chat = serverguide_model();
        let overwrite = PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: 6002,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            },
            allow_bits: Permissions::VIEW_CHANNEL.bits(),
            deny_bits: Permissions::SEND_MESSAGES.bits(),
        };
        read_only_chat
            .overwrites
            .insert(overwrite.key.clone(), overwrite);
        let blocked = build_server_guide_config(
            &live_serverguide_config_with_action_type(0),
            &read_only_chat,
        );
        assert!(blocked.config.is_none());
        assert!(blocked
            .blockers
            .iter()
            .any(|blocker| blocker.contains("nicht @everyone-sendbar")));
    }

    #[test]
    fn serverguide_put_payload_serialisiert_title_description_statt_name() {
        let config = build_server_guide_config(
            &json!({"enabled": false, "new_member_actions": []}),
            &serverguide_model(),
        )
        .config
        .expect("serverguide config");
        let payload = serde_json::to_value(&config).expect("payload");

        assert_json_key_absent_recursive(&payload, "name");
        assert_eq!(payload["new_member_actions"][0]["title"], "Sag Hallo");
        assert_eq!(payload["new_member_actions"][0]["description"], "");
        assert_eq!(payload["resource_channels"][0]["title"], "Regelwerk");
        assert_eq!(payload["resource_channels"][0]["description"], "");

        let parsed: ServerGuideConfig = serde_json::from_value(json!({
            "enabled": true,
            "welcome_message": {"author_ids": ["1"], "message": "Willkommen"},
            "new_member_actions": [{
                "channel_id": "6002",
                "action_type": 1,
                "title": "Live Action",
                "description": "Live Beschreibung"
            }],
            "resource_channels": [{
                "channel_id": "6004",
                "title": "Live Resource",
                "description": "Resource Beschreibung"
            }]
        }))
        .expect("GET response parses");
        assert_eq!(parsed.new_member_actions[0].title, "Live Action");
        assert_eq!(
            parsed.new_member_actions[0].description.as_deref(),
            Some("Live Beschreibung")
        );
        assert_eq!(parsed.resource_channels[0].title, "Live Resource");
        assert_eq!(
            parsed.resource_channels[0].description.as_deref(),
            Some("Resource Beschreibung")
        );
    }

    #[test]
    fn serverguide_diff_parst_get_response_mit_title() {
        let config = build_server_guide_config(
            &json!({"enabled": false, "new_member_actions": []}),
            &serverguide_model(),
        )
        .config
        .expect("serverguide config");
        let diff = serverguide_diff(
            GUILD_ID,
            &live_serverguide_config_with_action_type(SERVER_GUIDE_ACTION_TYPE_CHAT),
            &config,
        )
        .expect("serverguide diff");
        let actual = diff.changes[0].actual.as_ref().expect("actual");

        assert_eq!(actual["config"]["new_member_actions"][0]["title"], "Alt");
        assert_json_key_absent_recursive(&actual["config"], "name");
    }

    #[tokio::test]
    async fn regelwerk_discord_retry_retryt_429_mit_retry_after_body() {
        let attempts = Arc::new(Mutex::new(0usize));
        let attempts_for_sender = Arc::clone(&attempts);

        let response = discord_regelwerk_send_with_retry(
            move || {
                let attempts_for_call = Arc::clone(&attempts_for_sender);
                async move {
                    let mut attempts = attempts_for_call.lock().expect("attempts");
                    *attempts += 1;
                    if *attempts == 1 {
                        return Ok(DiscordRestResponse::for_test(
                            reqwest::StatusCode::TOO_MANY_REQUESTS,
                            json!({"retry_after": 0.0}).to_string(),
                        ));
                    }
                    Ok(DiscordRestResponse::for_test(
                        reqwest::StatusCode::OK,
                        json!({"ok": true}).to_string(),
                    ))
                }
            },
            "GET",
        )
        .await
        .expect("retry succeeds");
        let body: Value = discord_regelwerk_json_response(response, "GET").expect("json response");

        assert_eq!(body, json!({"ok": true}));
        assert_eq!(*attempts.lock().expect("attempts"), 2);
    }

    #[test]
    fn regelwerk_retry_after_nutzt_header_fallback() {
        let mut response =
            DiscordRestResponse::for_test(reqwest::StatusCode::TOO_MANY_REQUESTS, "{}".to_string());
        response.headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("0.25"),
        );

        let retry_after = regelwerk_retry_after(&response).expect("retry-after header");

        assert_eq!(retry_after.as_millis(), 250);
    }

    #[test]
    fn onboarding_builder_baut_drei_prompts_und_uebernimmt_rank_prompt_unveraendert() {
        let model = onboarding_model();
        let live = rank_live_onboarding_config();

        let built = build_welle2b_onboarding_config(&live, &model).expect("build");

        assert!(built.blockers.is_empty(), "blockers: {:?}", built.blockers);
        assert!(built.config.enabled);
        assert_eq!(built.config.mode, json!(1));
        // Live-Uebernahme: 9 gueltige Owner-IDs bleiben, die stale ID faellt raus.
        assert_eq!(
            built.config.default_channel_ids,
            (6001..=6009).map(|id| id.to_string()).collect::<Vec<_>>()
        );
        assert!(built
            .warnings
            .iter()
            .any(|warning| warning.contains("stale Live-IDs")));
        assert_eq!(built.config.prompts.len(), 3);
        assert_eq!(built.config.prompts[0].title, "Wo stehst du gerade?");
        assert!(built.config.prompts[0].single_select);
        assert!(built.config.prompts[0].required);
        assert!(built.config.prompts[0].in_onboarding);
        assert_eq!(
            built.config.prompts[1].title,
            "Wofür willst du Pings bekommen?"
        );
        assert!(!built.config.prompts[1].single_select);
        assert!(!built.config.prompts[1].required);
        assert_eq!(built.config.prompts[2].title, "Dein Rang");
        assert_eq!(built.config.prompts[2].id.as_deref(), Some("rank-prompt"));
        assert_eq!(built.config.prompts[2].options.len(), 12);
        assert_eq!(
            serde_json::to_value(&built.config.prompts[2]).expect("rank prompt"),
            live["prompts"][1]
        );
    }

    #[test]
    fn onboarding_payload_laesst_neue_ids_weg_und_nutzt_name_resolved_ids() {
        let model = onboarding_model();
        let live = rank_live_onboarding_config();

        let built = build_welle2b_onboarding_config(&live, &model).expect("build");
        let payload = serde_json::to_value(&built.config).expect("payload");

        let prompts = payload["prompts"].as_array().expect("prompts");
        assert_eq!(prompts.len(), 3);
        for prompt in [&prompts[0], &prompts[1]] {
            assert!(!prompt.as_object().expect("prompt").contains_key("id"));
            for option in prompt["options"].as_array().expect("options") {
                assert!(!option.as_object().expect("option").contains_key("id"));
            }
        }
        assert_eq!(prompts[2]["id"], "rank-prompt");
        assert_eq!(prompts[2]["options"][0]["id"], "rank-option-0");

        let weiche = &payload["prompts"][0]["options"];
        assert_eq!(
            weiche[0]["title"],
            "Ich spiele Deadlock und suche Mitspieler"
        );
        assert_eq!(weiche[0]["role_ids"], json!([]));
        assert_eq!(weiche[0]["channel_ids"], json!(["6003"]));
        assert_eq!(weiche[1]["role_ids"], json!(["5001"]));
        assert_eq!(weiche[2]["role_ids"], json!(["5002"]));

        let ping_options = payload["prompts"][1]["options"]
            .as_array()
            .expect("ping options");
        assert_eq!(ping_options[0]["title"], "Patchnotes");
        assert_eq!(ping_options[0]["role_ids"], json!(["5003"]));
        assert_eq!(ping_options[4]["title"], "Streams");
        assert_eq!(ping_options[4]["role_ids"], json!(["5007"]));
    }

    #[test]
    fn onboarding_put_payload_wandelt_get_emoji_objekte_in_put_emoji_felder() {
        let model = onboarding_model();
        let live = rank_live_onboarding_config();

        let built = build_welle2b_onboarding_config(&live, &model).expect("build");
        let payload =
            serde_json::to_value(native_onboarding_put_payload(&built.config)).expect("payload");

        assert_json_key_absent_recursive(&payload, "emoji");
        let prompts = payload["prompts"].as_array().expect("prompts");
        assert_eq!(prompts[0]["options"][0]["emoji_name"], "🎮");
        assert_eq!(prompts[0]["options"][0]["emoji_animated"], false);
        assert_eq!(prompts[0]["options"][1]["emoji_name"], "🔑");
        assert_eq!(prompts[0]["options"][2]["emoji_name"], "🌱");
        // Discord verlangt id auch fuer neue Prompts (BASE_TYPE_REQUIRED,
        // live verifiziert 2026-07-03): neue Objekte tragen Platzhalter-IDs.
        assert_eq!(prompts[0]["id"], "0");
        assert!(prompts[1]["id"].is_string());

        let rank_option = &prompts[2]["options"][0];
        assert_eq!(rank_option["id"], "rank-option-0");
        assert_eq!(rank_option["emoji_id"], "9000");
        assert_eq!(rank_option["emoji_name"], "ranked");
        assert_eq!(rank_option["emoji_animated"], true);
    }

    fn assert_json_key_absent_recursive(value: &Value, forbidden: &str) {
        match value {
            Value::Object(map) => {
                assert!(
                    !map.contains_key(forbidden),
                    "JSON enthaelt verbotenen Key `{forbidden}`: {value}"
                );
                for child in map.values() {
                    assert_json_key_absent_recursive(child, forbidden);
                }
            }
            Value::Array(values) => {
                for child in values {
                    assert_json_key_absent_recursive(child, forbidden);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn onboarding_ping_rollen_matching_ist_exakt_statt_token_fuzzy() {
        let mut model = onboarding_model();
        model
            .roles
            .retain(|_, role| role.name != "Patchnotes Ping Rolle");
        model.roles.insert(
            5999,
            onboarding_role(5999, "Deadlock Patchnotes Ping Rolle", true),
        );

        let built = build_welle2b_onboarding_config(&rank_live_onboarding_config(), &model)
            .expect("builder returns blockers");

        assert!(built
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Rolle `Patchnotes`")));
        assert!(built.config.prompts[1].options[0].role_ids.is_empty());
    }

    #[test]
    fn onboarding_apply_revalidiert_role_und_channel_ids_gegen_live_modell() {
        let mut config =
            build_welle2b_onboarding_config(&rank_live_onboarding_config(), &onboarding_model())
                .expect("build")
                .config;
        config.prompts[1].options[0].role_ids = vec!["999999".to_string()];
        config.prompts[0].options[0].channel_ids = vec!["kaputt".to_string()];

        let err = validate_onboarding_references(&onboarding_model(), &config)
            .expect_err("missing refs block apply");

        assert!(err.contains("Rolle `999999` fehlt"));
        assert!(err.contains("Kanal `kaputt` ist keine Discord-ID"));
    }

    #[test]
    fn onboarding_preview_extraktion_blockt_falschen_message_key() {
        let model = onboarding_model();
        let live = rank_live_onboarding_config();
        let config = build_welle2b_onboarding_config(&live, &model)
            .expect("build")
            .config;
        let mut diff = onboarding_diff(GUILD_ID, &live, &config).expect("diff");
        diff.changes[0].object.message_key = Some("anderer-key".to_string());

        let err = extract_onboarding_config_from_diff(&diff, GUILD_ID)
            .expect_err("wrong message_key must fail");

        assert!(err
            .to_string()
            .contains("keine Native-Onboarding-Zielconfig"));
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn onboarding_preview_load_bindet_guild_message_key_und_applied_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let model = onboarding_model();
        let live = rank_live_onboarding_config();
        let config = build_welle2b_onboarding_config(&live, &model)
            .expect("build")
            .config;
        let diff = onboarding_diff(GUILD_ID, &live, &config).expect("diff");

        let good_id = insert_raw_onboarding_preview(pool, GUILD_ID, &diff).await?;
        assert!(load_onboarding_preview(pool, good_id, GUILD_ID)
            .await
            .is_ok());

        sqlx::query(
            "UPDATE server_config.diff_previews
                SET created_at = now() - interval '16 minutes'
              WHERE preview_id = $1",
        )
        .bind(good_id)
        .execute(pool)
        .await?;
        let stale_err = load_onboarding_preview(pool, good_id, GUILD_ID)
            .await
            .expect_err("stale onboarding preview must fail");
        assert!(stale_err
            .to_string()
            .contains("Preview abgelaufen, bitte neu diffen"));

        sqlx::query(
            "UPDATE server_config.diff_previews
                SET created_at = now(), applied_at = now()
              WHERE preview_id = $1",
        )
        .bind(good_id)
        .execute(pool)
        .await?;
        assert!(load_onboarding_preview(pool, good_id, GUILD_ID)
            .await
            .is_err());

        let wrong_guild_id = insert_raw_onboarding_preview(pool, GUILD_ID + 1, &diff).await?;
        assert!(load_onboarding_preview(pool, wrong_guild_id, GUILD_ID)
            .await
            .is_err());

        let mut wrong_key = diff.clone();
        wrong_key.changes[0].object.message_key = Some("anderer-key".to_string());
        let wrong_key_id = insert_raw_onboarding_preview(pool, GUILD_ID, &wrong_key).await?;
        assert!(load_onboarding_preview(pool, wrong_key_id, GUILD_ID)
            .await
            .is_err());

        let serverguide_config = build_server_guide_config(
            &json!({"enabled": false, "new_member_actions": []}),
            &serverguide_model(),
        )
        .config
        .expect("serverguide config");
        let serverguide = serverguide_diff(
            GUILD_ID,
            &live_serverguide_config_with_action_type(SERVER_GUIDE_ACTION_TYPE_CHAT),
            &serverguide_config,
        )
        .expect("serverguide diff");
        let serverguide_id = insert_raw_onboarding_preview(pool, GUILD_ID, &serverguide).await?;
        sqlx::query(
            "UPDATE server_config.diff_previews
                SET created_at = now() - interval '16 minutes'
              WHERE preview_id = $1",
        )
        .bind(serverguide_id)
        .execute(pool)
        .await?;
        let stale_err = load_serverguide_preview(pool, serverguide_id, GUILD_ID)
            .await
            .expect_err("stale serverguide preview must fail");
        assert!(stale_err
            .to_string()
            .contains("Preview abgelaufen, bitte neu diffen"));

        Ok(())
    }

    async fn insert_raw_onboarding_preview(
        pool: &PgPool,
        guild_id: u64,
        diff: &ServerDiff,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let diff_hash = sha256_hex(&serde_json::to_vec(diff)?);
        let preview_id = sqlx::query_scalar(
            "INSERT INTO server_config.diff_previews
             (guild_id, diff_hash, diff_json, human_summary)
             VALUES ($1, $2, $3::text::jsonb, 'test')
             RETURNING preview_id",
        )
        .bind(i64::try_from(guild_id)?)
        .bind(diff_hash)
        .bind(serde_json::to_string(diff)?)
        .fetch_one(pool)
        .await?;
        Ok(preview_id)
    }

    #[test]
    fn onboarding_builder_listet_fehlende_rollen_und_kanaele_als_blocker() {
        let mut model = onboarding_model();
        model.roles.retain(|_, role| role.name != "Invite-Gast");
        model
            .channels
            .retain(|_, channel| channel.name != "server-support");

        let built = build_welle2b_onboarding_config(&rank_live_onboarding_config(), &model)
            .expect("builder returns blockers");

        assert!(built
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Rolle `Invite-Gast`")));
        // Verschwundene Default-Kanaele blocken nicht mehr (Live-Uebernahme),
        // sie fallen als stale ID mit Warnung raus.
        assert!(built
            .warnings
            .iter()
            .any(|warning| warning.contains("stale Live-IDs") && warning.contains("6009")));
    }

    #[test]
    fn onboarding_default_channel_validation_prueft_sieben_und_fuenf_sendefaehige() {
        let mut model = onboarding_model();
        let defaults = ["6001", "6002", "6003", "6004", "6005", "6006", "6007"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        validate_default_channels_7_5(&model, &defaults).expect("five writable defaults");

        for channel_id in [6001, 6002, 6003] {
            let overwrite = PermissionOverwriteSpec {
                guild_id: GUILD_ID,
                key: OverwriteKey {
                    channel_id,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                },
                allow_bits: Permissions::VIEW_CHANNEL.bits(),
                deny_bits: Permissions::SEND_MESSAGES.bits(),
            };
            model.overwrites.insert(overwrite.key.clone(), overwrite);
        }
        let err = validate_default_channels_7_5(&model, &defaults)
            .expect_err("less than five writable defaults must fail");
        assert!(err.contains("mindestens 5"));

        let too_few = defaults.into_iter().take(6).collect::<Vec<_>>();
        let err = validate_default_channels_7_5(&onboarding_model(), &too_few)
            .expect_err("less than seven defaults must fail");
        assert!(err.contains("mindestens 7"));
    }

    #[test]
    fn rollback_artifact_v2_wird_akzeptiert() {
        let value = v2_artifact_value();
        let hash = sha256_json_value(&value).expect("hash");
        let artifact = verify_rollback_artifact(7, &hash, value, GUILD_ID).expect("artifact");
        assert_eq!(artifact.version, ROLLBACK_VERSION);
        assert_eq!(artifact.member_role_assignments.len(), 1);
    }

    #[test]
    fn rollback_artifact_serialisiert_vec_dto_mit_overwrites_hashstabil() {
        let model = rollback_roundtrip_model();
        let value = serde_json::to_value(RollbackArtifact {
            version: ROLLBACK_VERSION.to_string(),
            guild_id: GUILD_ID,
            created_at: "2026-07-02T00:00:00.000Z".to_string(),
            snapshot_id: 10,
            structure_snapshot: RollbackGuildModel::from_model(&model),
            dynamic_namespaces: Vec::new(),
            documented_exceptions: Vec::new(),
            member_role_assignments: vec![MemberRoleAssignment {
                member_id: 1,
                role_ids: vec![2, 3],
            }],
            native_onboarding_config: json!({"enabled": true}),
        })
        .expect("artifact value");

        assert!(value["structure_snapshot"]["categories"].is_array());
        assert!(value["structure_snapshot"]["channels"].is_array());
        assert!(value["structure_snapshot"]["roles"].is_array());
        assert!(value["structure_snapshot"]["overwrites"].is_array());
        assert!(value["structure_snapshot"]["bot_messages"].is_array());

        let hash = sha256_json_value(&value).expect("hash");
        let roundtrip_value: Value =
            serde_json::from_str(&serde_json::to_string(&value).expect("json text"))
                .expect("roundtrip value");
        assert_eq!(
            sha256_json_value(&roundtrip_value).expect("roundtrip hash"),
            hash
        );

        let artifact =
            verify_rollback_artifact(7, &hash, roundtrip_value, GUILD_ID).expect("artifact");
        let restored = artifact
            .structure_snapshot
            .to_model()
            .expect("structure snapshot");
        assert_eq!(restored, model);
    }

    #[test]
    fn rollback_artifact_v1_wird_abgelehnt() {
        let err = verify_rollback_artifact(
            7,
            "ignored",
            json!({
                "version": ROLLBACK_VERSION_V1,
                "guild_id": GUILD_ID,
            }),
            GUILD_ID,
        )
        .expect_err("v1 must fail");
        assert_eq!(err.kind, ServerSyncErrorKind::BadRequest);
        assert!(err.to_string().contains("v1"));
        assert!(err.to_string().contains("v2"));
    }

    #[test]
    fn rollback_artifact_hash_mismatch_wird_abgelehnt() {
        let err = verify_rollback_artifact(7, "wrong-hash", v2_artifact_value(), GUILD_ID)
            .expect_err("hash mismatch must fail");
        assert_eq!(err.kind, ServerSyncErrorKind::BadRequest);
        assert!(err.to_string().contains("Hash-Mismatch"));
    }

    #[test]
    fn attachment_guard_erlaubt_exakt_10_mib_und_blockt_darueber() {
        let exact = vec![0; MAX_BRIDGE_ATTACHMENT_BYTES];
        let (attachments, notice) =
            attachment_or_db_notice("exact.json".to_string(), exact, "Artefakt", 7);
        assert_eq!(attachments.len(), 1);
        assert!(notice.is_empty());
        assert_eq!(attachments[0].data.len(), MAX_BRIDGE_ATTACHMENT_BYTES);

        let too_large = vec![0; MAX_BRIDGE_ATTACHMENT_BYTES + 1];
        let (attachments, notice) =
            attachment_or_db_notice("large.json".to_string(), too_large, "Artefakt", 7);
        assert!(attachments.is_empty());
        assert!(notice.contains("Artefakt liegt in der DB (id 7)"));
    }

    #[tokio::test]
    async fn apply_ohne_konfigurierten_token_liefert_403() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service, None);
        let response = app
            .oneshot(request(
                "/serversync/apply",
                Some("anything"),
                json!({"preview_id": 1, "hash": "h", "confirm": true}),
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["ok"], false);
    }

    #[tokio::test]
    async fn apply_ohne_header_liefert_403() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service, Some("secret".to_string()));
        let response = app
            .oneshot(request(
                "/serversync/apply",
                None,
                json!({"preview_id": 1, "hash": "h", "confirm": true}),
            ))
            .await
            .expect("response");
        let (status, _) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn apply_mit_kaputtem_json_ohne_header_liefert_403() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service, Some("secret".to_string()));
        let response = app
            .oneshot(request_raw("/serversync/apply", None, "{kaputt"))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["ok"], false);
    }

    #[tokio::test]
    async fn apply_parst_confirm_und_liefert_json() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));
        let response = app
            .oneshot(request(
                "/serversync/apply",
                Some("secret"),
                json!({"preview_id": 42, "hash": "abc", "confirm": true}),
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["apply_run_id"], 12);
        assert_eq!(
            service.apply_calls.lock().expect("apply calls").as_slice(),
            &[(42, "abc".to_string(), true, None)]
        );
    }

    #[tokio::test]
    async fn restore_http_erzeugt_preview() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));
        let response = app
            .oneshot(request(
                "/serversync/restore",
                Some("secret"),
                json!({"rollback_export_id": 99}),
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["preview_id"], 13);
        assert_eq!(body["result"]["rollback_export_id"], 99);
        assert_eq!(
            service
                .restore_calls
                .lock()
                .expect("restore calls")
                .as_slice(),
            &[(Some(99), None)]
        );
    }

    #[tokio::test]
    async fn diff_antwort_enthaelt_vollen_diff_text() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service, Some("secret".to_string()));
        let response = app
            .oneshot(request("/serversync/diff", Some("secret"), json!({})))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["diff_text"], "{\"changes\":[]}");
        assert_eq!(body["result"]["human_summary"], "summary");
    }

    #[tokio::test]
    async fn onboarding_preview_http_liefert_hash_und_blockerstruktur() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service, Some("secret".to_string()));
        let response = app
            .oneshot(request(
                "/serversync/onboarding-preview",
                Some("secret"),
                json!({}),
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["preview_id"], 21);
        assert_eq!(body["result"]["diff_hash"], "onboarding-hash");
        assert!(body["result"]["blockers"]
            .as_array()
            .expect("blockers")
            .is_empty());
    }

    #[tokio::test]
    async fn serverguide_preview_http_liefert_hash_und_apply_parst_confirm() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let preview = app
            .clone()
            .oneshot(request(
                "/serversync/serverguide-preview",
                Some("secret"),
                json!({}),
            ))
            .await
            .expect("preview response");
        let (status, body) = response_json(preview).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["preview_id"], 31);
        assert_eq!(body["result"]["diff_hash"], "serverguide-hash");

        let apply = app
            .oneshot(request(
                "/serversync/serverguide-apply",
                Some("secret"),
                json!({"preview_id": 31, "hash": "serverguide-hash", "confirm": true}),
            ))
            .await
            .expect("apply response");
        let (status, body) = response_json(apply).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["apply_run_id"], 32);
        assert_eq!(
            service
                .serverguide_apply_calls
                .lock()
                .expect("serverguide apply calls")
                .as_slice(),
            &[(31, "serverguide-hash".to_string(), true, None)]
        );
    }

    #[tokio::test]
    async fn archive_enable_disable_http_toggelt_flag() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let enable = app
            .clone()
            .oneshot(request(
                "/serversync/archive-enable",
                Some("secret"),
                json!({}),
            ))
            .await
            .expect("enable response");
        let (status, body) = response_json(enable).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["enabled"], true);

        let disable = app
            .oneshot(request(
                "/serversync/archive-disable",
                Some("secret"),
                json!({}),
            ))
            .await
            .expect("disable response");
        let (status, body) = response_json(disable).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["enabled"], false);
        assert_eq!(
            service
                .archive_calls
                .lock()
                .expect("archive calls")
                .as_slice(),
            &[true, false]
        );
    }

    #[tokio::test]
    async fn regelwerk_publish_http_ist_dry_run_default_und_parst_confirm() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw(
                "/serversync/regelwerk-publish",
                Some("secret"),
                "",
            ))
            .await
            .expect("dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["threads_deleted"], 0);
        assert_eq!(
            body["result"]["bot_message_embed_titles"][0],
            "Hier starten ➜"
        );

        let confirmed = app
            .oneshot(request(
                "/serversync/regelwerk-publish",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["edited_message_id"], 7003);
        assert_eq!(
            service
                .regelwerk_calls
                .lock()
                .expect("regelwerk calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn regelwerk_apply_http_ist_dry_run_default_und_liefert_v2_payload() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw(
                "/serversync/regelwerk-apply",
                Some("secret"),
                "",
            ))
            .await
            .expect("regelwerk dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["messages"][0]["action"], "planned_edit");
        assert_eq!(
            body["result"]["payload_format"],
            regelwerk_publish::REGELWERK_PAYLOAD_FORMAT
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["flags"],
            regelwerk_publish::REGELWERK_COMPONENTS_V2_FLAG
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["components"][0]["items"][0]["media"]["url"],
            "attachment://regelwerk-hero.png"
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["components"][1]["components"][1]
                ["components"][0]["custom_id"],
            regelwerk_publish::REGELWERK_VERHALTEN_CUSTOM_ID
        );

        let confirmed = app
            .oneshot(request(
                "/serversync/regelwerk-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("regelwerk confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["messages"][0]["action"], "no_op");
        assert_eq!(
            body["result"]["deleted_legacy_message_ids"][0],
            regelwerk_publish::REGELWERK_OLD_MESSAGE_ID
        );
        assert_eq!(
            service
                .regelwerk_apply_calls
                .lock()
                .expect("regelwerk apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn support_apply_http_ist_dry_run_default_und_liefert_v2_payload() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw("/serversync/support-apply", Some("secret"), ""))
            .await
            .expect("support dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["messages"][0]["action"], "planned_edit");
        assert_eq!(
            body["result"]["payload_format"],
            support_publish::SUPPORT_PAYLOAD_FORMAT
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["flags"],
            support_publish::SUPPORT_COMPONENTS_V2_FLAG
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["components"][1]["components"][1]
                ["components"][0]["url"],
            support_publish::SUPPORT_DONATE_URL_DEFAULT
        );

        let confirmed = app
            .oneshot(request(
                "/serversync/support-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("support confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["messages"][0]["action"], "no_op");
        assert_eq!(
            service
                .support_apply_calls
                .lock()
                .expect("support apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn faq_apply_http_ist_dry_run_default_und_liefert_v2_select_payload() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw("/serversync/faq-apply", Some("secret"), ""))
            .await
            .expect("faq dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["messages"][0]["action"], "planned_edit");
        assert_eq!(
            body["result"]["payload_format"],
            faq_publish::FAQ_PAYLOAD_FORMAT
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["flags"],
            faq_publish::FAQ_COMPONENTS_V2_FLAG
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        let select = &body["result"]["messages"][0]["payload"]["components"][1]["components"][1]
            ["components"][0];
        assert_eq!(select["custom_id"], faq_publish::FAQ_SELECT_CUSTOM_ID);
        assert_eq!(select["placeholder"], faq_publish::FAQ_SELECT_PLACEHOLDER);
        assert_eq!(select["options"].as_array().expect("options").len(), 15);
        assert_eq!(select["options"][0]["value"], "f1");
        assert_eq!(select["options"][14]["value"], "f15");

        let confirmed = app
            .oneshot(request(
                "/serversync/faq-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("faq confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["messages"][0]["action"], "no_op");
        assert_eq!(
            service
                .faq_apply_calls
                .lock()
                .expect("faq apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn welcome_preview_http_liefert_komplette_payload_shape() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service, Some("secret".to_string()));

        let response = app
            .oneshot(request_raw(
                "/serversync/welcome-preview",
                Some("secret"),
                "",
            ))
            .await
            .expect("welcome preview response");
        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["channel_name"], "🧭willkommen");
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["sections"][0]["section_id"], "hero");
        assert_eq!(
            body["result"]["sections"][0]["payload"]["flags"],
            welcome_publish::WELCOME_COMPONENTS_V2_FLAG
        );
        assert!(body["result"]["sections"][0]["payload"]["content"].is_null());
        assert!(body["result"]["sections"][0]["payload"]["embeds"].is_null());
        assert_eq!(
            body["result"]["sections"][0]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        assert_eq!(
            body["result"]["sections"][0]["payload"]["components"][0]["components"][0]["content"],
            "Platzhalter"
        );
        assert_eq!(
            body["result"]["team_roles"][0]["matched_role_name"],
            "Community Moderator"
        );
        assert_eq!(body["result"]["sections"][0]["banner"]["present"], false);
    }

    #[tokio::test]
    async fn welcome_apply_http_ist_dry_run_default_und_parst_confirm() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw("/serversync/welcome-apply", Some("secret"), ""))
            .await
            .expect("welcome dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["sections"][0]["action"], "planned_edit");

        let confirmed = app
            .oneshot(request(
                "/serversync/welcome-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("welcome confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["sections"][0]["action"], "edited");
        assert_eq!(body["result"]["edited_message_ids"]["hero"][0], 7001);
        assert_eq!(
            service
                .welcome_apply_calls
                .lock()
                .expect("welcome apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn rang_guide_apply_http_ist_dry_run_default_und_liefert_v2_payload() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw(
                "/serversync/rang-guide-apply",
                Some("secret"),
                "",
            ))
            .await
            .expect("rang guide dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["messages"][0]["action"], "planned_edit");
        assert_eq!(
            body["result"]["payload_format"],
            rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["flags"],
            rang_guide_publish::RANG_GUIDE_COMPONENTS_V2_FLAG
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["components"][0]["components"][0]["items"][0]
                ["media"]["url"],
            "attachment://rang-guide-hero.png"
        );
        assert_eq!(
            body["result"]["messages"][0]["payload"]["components"][1]["components"][1]
                ["components"][0]["custom_id"],
            rang_guide_publish::STEAM_LINK_OPEN_CUSTOM_ID
        );
        assert_eq!(
            body["result"]["legacy_cleanup_candidates"][0]["message_id"],
            8100
        );
        assert!(body["result"]["deleted_legacy_message_ids"]
            .as_array()
            .expect("deleted legacy")
            .is_empty());

        let confirmed = app
            .oneshot(request(
                "/serversync/rang-guide-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("rang guide confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["messages"][0]["action"], "no_op");
        assert_eq!(body["result"]["deleted_legacy_message_ids"][0], 8100);
        assert_eq!(
            service
                .rang_guide_apply_calls
                .lock()
                .expect("rang guide apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn service_v2_preview_macht_keine_writes() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_rang_repo(repo.path(), None);
        let fake = spawn_fake_discord(42, Vec::new(), Vec::new()).await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.rang_guide_apply(false).await.expect("preview");

        assert!(output.dry_run);
        assert!(fake.state.post_calls.lock().expect("post calls").is_empty());
        assert!(fake
            .state
            .patch_calls
            .lock()
            .expect("patch calls")
            .is_empty());
        assert!(fake
            .state
            .delete_calls
            .lock()
            .expect("delete calls")
            .is_empty());
        assert_eq!(
            get_serversync_kv(db.pool(), &rang_guide_publish::rang_guide_message_id_key(0)).await,
            None
        );
        assert_eq!(
            get_serversync_kv(db.pool(), rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY).await,
            None
        );
    }

    #[tokio::test]
    async fn service_v2_repost_teilfehler_laesst_altes_kv_und_alte_messages_stehen() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        let long_step2 = "x".repeat(2700);
        write_test_rang_repo(repo.path(), Some(&long_step2));
        let planned = rang_guide_publish::build_rang_guide_publish_output(
            repo.path(),
            &[9101, 9102, 9103],
            Some("1"),
            Some("old-hash"),
            true,
        )
        .expect("planned");
        assert!(
            planned.messages.len() >= 2,
            "test setup needs at least two posts"
        );
        for (index, message_id) in [9101_u64, 9102, 9103]
            .into_iter()
            .take(planned.messages.len())
            .enumerate()
        {
            set_serversync_kv(
                db.pool(),
                &rang_guide_publish::rang_guide_message_id_key(index),
                &message_id.to_string(),
            )
            .await;
        }
        set_serversync_kv(
            db.pool(),
            rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY,
            "1",
        )
        .await;
        set_serversync_kv(
            db.pool(),
            rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY,
            "old-hash",
        )
        .await;
        let old_messages = [9101_u64, 9102, 9103]
            .into_iter()
            .zip(planned.messages.iter())
            .map(|(message_id, message)| {
                fake_discord_message(
                    message_id,
                    42,
                    rang_guide_publish::RANG_GUIDE_COMPONENTS_V2_FLAG,
                    message.payload.components.clone(),
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        let fake = spawn_fake_discord(
            42,
            old_messages,
            vec![
                FakePostResponse::Ok(9201),
                FakePostResponse::Status(StatusCode::INTERNAL_SERVER_ERROR),
            ],
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let err = service
            .rang_guide_apply(true)
            .await
            .expect_err("partial repost must fail");

        assert!(err.to_string().contains("Discord POST fehlgeschlagen"));
        assert_eq!(
            fake.state
                .delete_calls
                .lock()
                .expect("delete calls")
                .as_slice(),
            &[9201]
        );
        assert_eq!(
            get_serversync_kv(db.pool(), &rang_guide_publish::rang_guide_message_id_key(0))
                .await
                .as_deref(),
            Some("9101")
        );
        assert_eq!(
            get_serversync_kv(db.pool(), &rang_guide_publish::rang_guide_message_id_key(1))
                .await
                .as_deref(),
            Some("9102")
        );
        assert_eq!(
            get_serversync_kv(db.pool(), &rang_guide_publish::rang_guide_message_id_key(2))
                .await
                .as_deref(),
            Some("9103")
        );
        assert_eq!(
            get_serversync_kv(db.pool(), rang_guide_publish::RANG_GUIDE_PAYLOAD_HASH_KEY)
                .await
                .as_deref(),
            Some("old-hash")
        );
    }

    #[tokio::test]
    async fn service_v2_stored_delete_loescht_fremd_autor_message_nicht() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_rang_repo(repo.path(), None);
        set_serversync_kv(
            db.pool(),
            &rang_guide_publish::rang_guide_message_id_key(0),
            "9101",
        )
        .await;
        set_serversync_kv(
            db.pool(),
            rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY,
            "1",
        )
        .await;
        let fake = spawn_fake_discord(
            42,
            vec![fake_discord_message(
                9101,
                99,
                rang_guide_publish::RANG_GUIDE_COMPONENTS_V2_FLAG,
                Vec::new(),
                Vec::new(),
            )],
            vec![FakePostResponse::Ok(9201)],
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.rang_guide_apply(true).await.expect("apply");

        assert!(output.deleted_message_ids.is_empty());
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("passt nicht zur eigenen V2-Signatur")));
        assert!(fake
            .state
            .delete_calls
            .lock()
            .expect("delete calls")
            .is_empty());
        assert_eq!(
            get_serversync_kv(db.pool(), &rang_guide_publish::rang_guide_message_id_key(0))
                .await
                .as_deref(),
            Some("9201")
        );
    }

    #[tokio::test]
    async fn service_regelwerk_v2_stored_delete_loescht_fremd_autor_message_nicht() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_regelwerk_repo(repo.path());
        set_serversync_kv(
            db.pool(),
            &regelwerk_publish::regelwerk_message_id_key(0),
            "9101",
        )
        .await;
        set_serversync_kv(
            db.pool(),
            regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY,
            "1",
        )
        .await;
        let fake = spawn_fake_discord(
            42,
            vec![fake_discord_message(
                9101,
                99,
                regelwerk_publish::REGELWERK_COMPONENTS_V2_FLAG,
                Vec::new(),
                Vec::new(),
            )],
            vec![FakePostResponse::Ok(9201)],
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.regelwerk_apply(true).await.expect("apply");

        assert!(output.deleted_message_ids.is_empty());
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("passt nicht zur eigenen V2-Signatur")));
        assert!(fake
            .state
            .delete_calls
            .lock()
            .expect("delete calls")
            .is_empty());
        assert_eq!(
            get_serversync_kv(db.pool(), &regelwerk_publish::regelwerk_message_id_key(0))
                .await
                .as_deref(),
            Some("9201")
        );
    }

    #[tokio::test]
    async fn service_regelwerk_v2_loescht_alte_bot_message_nach_erfolgreichem_publish() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_regelwerk_repo(repo.path());
        let fake = spawn_fake_discord(
            42,
            vec![fake_discord_message_with_content(
                regelwerk_publish::REGELWERK_OLD_MESSAGE_ID,
                42,
                0,
                Vec::new(),
                Vec::new(),
                "**📜 Regelwerk · alt",
            )],
            vec![FakePostResponse::Ok(9201)],
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.regelwerk_apply(true).await.expect("apply");

        assert_eq!(output.posted_message_ids, vec![9201]);
        assert_eq!(
            output.deleted_legacy_message_ids,
            vec![regelwerk_publish::REGELWERK_OLD_MESSAGE_ID]
        );
        assert_eq!(
            fake.state
                .delete_calls
                .lock()
                .expect("delete calls")
                .as_slice(),
            &[regelwerk_publish::REGELWERK_OLD_MESSAGE_ID]
        );
    }

    #[tokio::test]
    async fn service_support_v2_owner_message_bleibt_unangetastet() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_support_repo(repo.path());
        set_serversync_kv(
            db.pool(),
            &support_publish::support_message_id_key(0),
            &support_publish::SUPPORT_OWNER_MESSAGE_ID.to_string(),
        )
        .await;
        set_serversync_kv(db.pool(), support_publish::SUPPORT_PAYLOAD_FORMAT_KEY, "1").await;
        let fake = spawn_fake_discord(
            42,
            vec![fake_discord_message(
                support_publish::SUPPORT_OWNER_MESSAGE_ID,
                99,
                support_publish::SUPPORT_COMPONENTS_V2_FLAG,
                Vec::new(),
                Vec::new(),
            )],
            vec![FakePostResponse::Ok(9201)],
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.support_apply(true).await.expect("apply");

        assert!(output.deleted_message_ids.is_empty());
        assert!(!fake
            .state
            .delete_calls
            .lock()
            .expect("delete calls")
            .contains(&support_publish::SUPPORT_OWNER_MESSAGE_ID));
        assert_eq!(
            get_serversync_kv(db.pool(), &support_publish::support_message_id_key(0))
                .await
                .as_deref(),
            Some("9201")
        );
    }

    #[tokio::test]
    async fn service_faq_v2_repost_loescht_fremde_message_nicht() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_faq_repo(repo.path());
        set_serversync_kv(db.pool(), &faq_publish::faq_message_id_key(0), "9501").await;
        set_serversync_kv(db.pool(), faq_publish::FAQ_PAYLOAD_FORMAT_KEY, "1").await;
        let fake = spawn_fake_discord(
            42,
            vec![fake_discord_message(
                9501,
                99,
                faq_publish::FAQ_COMPONENTS_V2_FLAG,
                Vec::new(),
                Vec::new(),
            )],
            vec![FakePostResponse::Ok(9601)],
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.faq_apply(true).await.expect("apply");

        assert_eq!(output.posted_message_ids, vec![9601]);
        assert!(output.deleted_message_ids.is_empty());
        assert!(fake
            .state
            .delete_calls
            .lock()
            .expect("delete calls")
            .is_empty());
        assert_eq!(
            get_serversync_kv(db.pool(), &faq_publish::faq_message_id_key(0))
                .await
                .as_deref(),
            Some("9601")
        );
    }

    #[tokio::test]
    async fn service_v2_kv_verlust_adoptiert_history_statt_neu_post() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_rang_repo(repo.path(), None);
        let planned = rang_guide_publish::build_rang_guide_publish_output(
            repo.path(),
            &[],
            Some(rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("planned");
        assert_eq!(planned.messages.len(), 1);
        let fake = spawn_fake_discord(
            42,
            vec![fake_discord_message(
                9101,
                42,
                rang_guide_publish::RANG_GUIDE_COMPONENTS_V2_FLAG,
                planned.messages[0].payload.components.clone(),
                Vec::new(),
            )],
            Vec::new(),
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.rang_guide_apply(true).await.expect("apply");

        assert_eq!(output.messages[0].action, "adopted_edit");
        assert_eq!(output.edited_message_ids, vec![9101]);
        assert!(fake.state.post_calls.lock().expect("post calls").is_empty());
        assert_eq!(
            fake.state
                .patch_calls
                .lock()
                .expect("patch calls")
                .as_slice(),
            &[9101]
        );
        assert_eq!(
            get_serversync_kv(db.pool(), &rang_guide_publish::rang_guide_message_id_key(0))
                .await
                .as_deref(),
            Some("9101")
        );
    }

    #[tokio::test]
    async fn service_voice_ux_apply_zieht_legacy_kv_auf_neue_zielkanaele_um() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_voice_ux_repo(repo.path(), b"voice-ux-banner");
        dl_central_db::kv::set(
            db.pool(),
            dl_voice::router::ROUTER_PANEL_KV_NS,
            dl_voice::router::ROUTER_PANEL_MESSAGE_KEY,
            &voice_ux_publish::VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID.to_string(),
        )
        .await
        .expect("set router legacy kv");
        dl_central_db::kv::set(
            db.pool(),
            dl_voice::lfg_panel::LFG_PANEL_KV_NS,
            dl_voice::lfg_panel::LFG_PANEL_MESSAGE_KEY,
            &voice_ux_publish::VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID.to_string(),
        )
        .await
        .expect("set lfg legacy kv");
        let fake = spawn_fake_discord(
            42,
            vec![
                fake_discord_message(
                    voice_ux_publish::VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID,
                    42,
                    0,
                    vec![json!({
                        "type": 1,
                        "components": [{
                            "type": 2,
                            "custom_id": "router_spawn_casual",
                        }],
                    })],
                    Vec::new(),
                ),
                fake_discord_message(
                    voice_ux_publish::VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID,
                    42,
                    0,
                    vec![json!({
                        "type": 1,
                        "components": [{
                            "type": 2,
                            "custom_id": dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID,
                        }],
                    })],
                    Vec::new(),
                ),
            ],
            (9201_u64..=9209).map(FakePostResponse::Ok).collect(),
        )
        .await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.router_apply(true).await.expect("apply");

        assert_eq!(output.targets.len(), 2);
        assert_eq!(output.targets[0].posted_message_ids, vec![9201, 9202]);
        assert_eq!(output.targets[1].posted_message_ids, vec![9203, 9204]);
        assert_eq!(output.forum_post.thread_id, Some(9205));
        assert!(output.forum_post.pinned);
        assert_eq!(
            output.deleted_legacy_message_ids,
            vec![
                voice_ux_publish::VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID,
                voice_ux_publish::VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID,
            ]
        );
        for (channel_id, expected_ids) in [
            (voice_ux_publish::VOICE_UX_CHANNEL_ID, vec![9201_u64, 9202]),
            (
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                vec![9203_u64, 9204],
            ),
        ] {
            let prefix = voice_ux_publish::voice_ux_message_id_prefix(channel_id);
            for (index, expected_id) in expected_ids.into_iter().enumerate() {
                assert_eq!(
                    get_serversync_kv(db.pool(), &format!("{prefix}{index}")).await,
                    Some(expected_id.to_string())
                );
            }
            assert_eq!(
                get_serversync_kv(
                    db.pool(),
                    &voice_ux_publish::voice_ux_payload_format_key(channel_id)
                )
                .await
                .as_deref(),
                Some(voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT)
            );
            assert!(get_serversync_kv(
                db.pool(),
                &voice_ux_publish::voice_ux_payload_hash_key(channel_id)
            )
            .await
            .is_some());
        }
        assert_eq!(
            get_serversync_kv(
                db.pool(),
                voice_ux_publish::VOICE_UX_LFG_FORUM_THREAD_ID_KEY
            )
            .await
            .as_deref(),
            Some("9205")
        );
        assert!(dl_central_db::kv::get(
            db.pool(),
            dl_voice::router::ROUTER_PANEL_KV_NS,
            dl_voice::router::ROUTER_PANEL_MESSAGE_KEY,
        )
        .await
        .expect("get router legacy kv")
        .is_none());
        assert!(dl_central_db::kv::get(
            db.pool(),
            dl_voice::lfg_panel::LFG_PANEL_KV_NS,
            dl_voice::lfg_panel::LFG_PANEL_MESSAGE_KEY,
        )
        .await
        .expect("get lfg legacy kv")
        .is_none());
        assert_eq!(
            fake.state.post_calls.lock().expect("post calls").as_slice(),
            &[
                voice_ux_publish::VOICE_UX_CHANNEL_ID,
                voice_ux_publish::VOICE_UX_CHANNEL_ID,
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                voice_ux_publish::VOICE_UX_LFG_FORUM_CHANNEL_ID,
            ]
        );
        assert_eq!(output.targets[0].pinned_message_ids, vec![9201, 9202]);
        assert_eq!(output.targets[1].pinned_message_ids, vec![9203, 9204]);
        assert_eq!(
            fake.state.pin_calls.lock().expect("pin calls").as_slice(),
            &[
                (voice_ux_publish::VOICE_UX_CHANNEL_ID, 9201),
                (voice_ux_publish::VOICE_UX_CHANNEL_ID, 9202),
                (voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID, 9203),
                (voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID, 9204),
            ]
        );
        let post_bodies = fake.state.post_bodies.lock().expect("post bodies");
        let forum_create = post_bodies
            .iter()
            .find(|(channel_id, _)| *channel_id == voice_ux_publish::VOICE_UX_LFG_FORUM_CHANNEL_ID)
            .expect("forum create body");
        assert_eq!(
            forum_create.1["applied_tags"],
            json!([voice_ux_publish::VOICE_UX_LFG_FORUM_INFO_TAG_ID])
        );
        assert_eq!(
            fake.state
                .patched_channels
                .lock()
                .expect("patched channels")
                .as_slice(),
            &[(9205, json!({"pinned": true}))]
        );
    }

    #[tokio::test]
    async fn service_voice_ux_noop_zieht_fehlende_router_vc_pins_nach() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        write_test_voice_ux_repo(repo.path(), b"voice-ux-banner");
        let ids = BTreeMap::from([
            (voice_ux_publish::VOICE_UX_CHANNEL_ID, vec![9301, 9302]),
            (
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                vec![9311, 9312],
            ),
        ]);
        let formats = BTreeMap::from([
            (
                voice_ux_publish::VOICE_UX_CHANNEL_ID,
                Some(voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT.to_string()),
            ),
            (
                voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                Some(voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT.to_string()),
            ),
        ]);
        let planned = voice_ux_publish::build_voice_ux_publish_output(
            repo.path(),
            &ids,
            &formats,
            &BTreeMap::new(),
            Some(9401),
            None,
            true,
        )
        .expect("planned");
        for target in &planned.targets {
            let prefix = voice_ux_publish::voice_ux_message_id_prefix(target.channel_id);
            for (index, message_id) in target.stored_message_ids.iter().copied().enumerate() {
                set_serversync_kv(
                    db.pool(),
                    &format!("{prefix}{index}"),
                    &message_id.to_string(),
                )
                .await;
            }
            set_serversync_kv(
                db.pool(),
                &voice_ux_publish::voice_ux_payload_format_key(target.channel_id),
                voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT,
            )
            .await;
            set_serversync_kv(
                db.pool(),
                &voice_ux_publish::voice_ux_payload_hash_key(target.channel_id),
                &target.payload_hash,
            )
            .await;
        }
        set_serversync_kv(
            db.pool(),
            voice_ux_publish::VOICE_UX_LFG_FORUM_THREAD_ID_KEY,
            "9401",
        )
        .await;
        set_serversync_kv(
            db.pool(),
            voice_ux_publish::VOICE_UX_LFG_FORUM_PAYLOAD_HASH_KEY,
            &planned.forum_post.payload_hash,
        )
        .await;

        let mut messages = Vec::new();
        for target in &planned.targets {
            for message in &target.messages {
                let mut fake = fake_discord_message(
                    message.message_id.expect("message id"),
                    42,
                    voice_ux_publish::VOICE_UX_COMPONENTS_V2_FLAG,
                    message.payload.components.clone(),
                    Vec::new(),
                );
                fake["pinned"] = json!(target.channel_id == voice_ux_publish::VOICE_UX_CHANNEL_ID);
                messages.push(fake);
            }
        }
        let fake = spawn_fake_discord(42, messages, Vec::new()).await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.router_apply(true).await.expect("apply");

        assert!(fake.state.post_calls.lock().expect("post calls").is_empty());
        assert!(fake
            .state
            .patch_calls
            .lock()
            .expect("patch calls")
            .is_empty());
        assert_eq!(output.targets[0].messages[0].action, "no_op");
        assert_eq!(output.targets[1].messages[0].action, "no_op");
        assert_eq!(
            fake.state.pin_calls.lock().expect("pin calls").as_slice(),
            &[
                (voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID, 9311),
                (voice_ux_publish::VOICE_UX_ROUTER_CHAT_CHANNEL_ID, 9312),
            ]
        );
        assert_eq!(output.targets[1].pinned_message_ids, vec![9311, 9312]);
    }

    #[tokio::test]
    async fn service_lfg_panel_apply_blockt_alten_einzelpanel_publisher() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let repo = tempfile::tempdir().expect("repo");
        let fake = spawn_fake_discord(42, Vec::new(), Vec::new()).await;
        let service = ServerSyncService::new_for_test(
            db.pool().clone(),
            fake.base_url.clone(),
            repo.path().to_path_buf(),
        );

        let output = service.lfg_panel_apply(true).await.expect("blocked");

        assert_eq!(output.action, "blocked_superseded_by_voice_ux");
        assert_eq!(
            output.blocked_reason.as_deref(),
            Some("lfg_panel_apply_superseded_by_serversync_router_apply")
        );
        assert!(fake.state.post_calls.lock().expect("post calls").is_empty());
    }

    #[tokio::test]
    async fn router_apply_http_ist_dry_run_default_und_liefert_v2_payload() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw("/serversync/router-apply", Some("secret"), ""))
            .await
            .expect("router dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        let targets = body["result"]["targets"].as_array().expect("targets");
        assert_eq!(targets.len(), 2);
        let first_target = &targets[0];
        assert_eq!(
            first_target["channel_id"],
            voice_ux_publish::VOICE_UX_CHANNEL_ID
        );
        let messages = first_target["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["message_key"], "guide_lfg");
        assert_eq!(messages[1]["message_key"], "spawn_manage");
        assert!(messages
            .iter()
            .all(|message| message["action"] == "planned_edit"));
        assert_eq!(
            messages[0]["payload"]["flags"],
            voice_ux_publish::VOICE_UX_COMPONENTS_V2_FLAG
        );
        for (message, filenames) in messages.iter().zip([
            [
                voice_ux_publish::VOICE_UX_GUIDE_BANNER_FILENAME,
                voice_ux_publish::VOICE_UX_LFG_BANNER_FILENAME,
            ],
            [
                voice_ux_publish::VOICE_UX_SPAWN_BANNER_FILENAME,
                voice_ux_publish::VOICE_UX_MANAGE_BANNER_FILENAME,
            ],
        ]) {
            assert_eq!(
                message["payload"]["allowed_mentions"]["parse"]
                    .as_array()
                    .expect("parse")
                    .len(),
                0
            );
            let combined_container = &message["payload"]["components"][0];
            assert_eq!(combined_container["type"], json!(17));
            assert_eq!(
                combined_container["accent_color"],
                json!(voice_ux_publish::VOICE_UX_ACCENT_GOLD)
            );
            let galleries: Vec<&serde_json::Value> = combined_container["components"]
                .as_array()
                .expect("container children")
                .iter()
                .filter(|child| child["type"] == json!(12))
                .collect();
            assert_eq!(galleries.len(), 2);
            for (section, filename) in filenames.iter().enumerate() {
                assert_eq!(
                    message["payload"]["attachments"][section]["filename"],
                    *filename
                );
                assert_eq!(
                    galleries[section]["items"][0]["media"]["url"],
                    format!("attachment://{filename}")
                );
            }
        }
        let guide_text = messages[0]["payload"]["components"][0]["components"][1]["content"]
            .as_str()
            .expect("guide text");
        assert_eq!(guide_text, dl_voice::router::VOICE_GUIDE_TITLE);
        assert!(!guide_text.contains(dl_voice::router::VOICE_GUIDE_BODY));
        assert_eq!(
            messages[0]["payload"]["components"][0]["components"][2]["components"][0]["custom_id"],
            "tv_prefs_open"
        );
        assert_eq!(
            messages[0]["payload"]["components"][0]["components"][2]["components"][1]["custom_id"],
            "voice:guide:detail"
        );
        // Mitspieler-finden-Sektion hat keine Buttons mehr — nur den Link auf den Forum-Post.
        let lfg_text = messages[0]["payload"]["components"][0]["components"][5]["content"]
            .as_str()
            .expect("lfg text");
        assert!(lfg_text.contains("discord.com/channels/"));
        assert_eq!(
            messages[1]["payload"]["components"][0]["components"][2]["components"][0]["custom_id"],
            "router_spawn_casual"
        );
        let mode_buttons = messages[1]["payload"]["components"][0]["components"][2]["components"]
            .as_array()
            .expect("mode buttons");
        assert!(mode_buttons.iter().all(|button| button["style"] == 2));
        // Verwalten-Reihe endet ohne ⚙️ Voreinstellungen bei Rang-Gate.
        let lane_row = messages[1]["payload"]["components"][0]["components"][8]["components"]
            .as_array()
            .expect("lane row");
        assert_eq!(lane_row.len(), 5);
        assert_eq!(lane_row[3]["custom_id"], "tv_mode_switch_btn");
        assert_eq!(lane_row[4]["custom_id"], "tv_rank_gate");
        assert_eq!(
            body["result"]["forum_post"]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        assert_eq!(
            body["result"]["forum_post"]["payload"]["components"][0]["components"][1]["components"]
                [0]["custom_id"],
            dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID
        );

        let confirmed = app
            .oneshot(request(
                "/serversync/router-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("router confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(
            body["result"]["targets"][0]["messages"][0]["action"],
            "edited"
        );
        assert_eq!(body["result"]["forum_post"]["action"], "edited");
        assert_eq!(
            service
                .router_apply_calls
                .lock()
                .expect("router apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn lfg_panel_apply_http_ist_dry_run_default_und_liefert_shell_payload() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));

        let dry_run = app
            .clone()
            .oneshot(request_raw(
                "/serversync/lfg-panel-apply",
                Some("secret"),
                "",
            ))
            .await
            .expect("lfg dry-run response");
        let (status, body) = response_json(dry_run).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], true);
        assert_eq!(body["result"]["action"], "planned_edit");
        assert_eq!(
            body["result"]["payload"]["flags"],
            dl_voice::lfg_panel::LFG_COMPONENTS_V2_FLAG
        );
        assert_eq!(
            body["result"]["payload"]["allowed_mentions"]["parse"]
                .as_array()
                .expect("parse")
                .len(),
            0
        );
        assert_eq!(
            body["result"]["payload"]["components"][0]["components"][1]["content"],
            dl_voice::lfg_panel::LFG_PANEL_BODY
        );
        assert_eq!(
            body["result"]["payload"]["components"][0]["components"][2]["components"][0]
                ["custom_id"],
            dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID
        );

        let confirmed = app
            .oneshot(request(
                "/serversync/lfg-panel-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("lfg confirm response");
        let (status, body) = response_json(confirmed).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["dry_run"], false);
        assert_eq!(body["result"]["action"], "edited");
        assert_eq!(
            service
                .lfg_panel_apply_calls
                .lock()
                .expect("lfg panel apply calls")
                .as_slice(),
            &[false, true]
        );
    }

    #[tokio::test]
    async fn lfg_panel_apply_http_liefert_blocked_reason_statt_500() {
        let service = Arc::new(MockServerSync::default());
        *service
            .lfg_panel_apply_override
            .lock()
            .expect("lfg panel apply override") = Some(LfgPanelApplyOutput {
            guild_id: GUILD_ID,
            channel_id: Some(8101),
            dry_run: true,
            payload_format: dl_voice::lfg_panel::LFG_PAYLOAD_FORMAT.to_string(),
            stored_payload_format: None,
            stored_message_id: None,
            action: "blocked_invalid_channel_type".to_string(),
            message_id: None,
            blocked_reason: Some(
                "DL_LFG_PANEL_CHANNEL_ID zeigt auf forum-Kanal; Panel-Ziel muss ein Text-Kanal sein"
                    .to_string(),
            ),
            warnings: Vec::new(),
            payload: json!({"flags": dl_voice::lfg_panel::LFG_COMPONENTS_V2_FLAG}),
        });
        let app = router(service.clone(), Some("secret".to_string()));

        let response = app
            .oneshot(request(
                "/serversync/lfg-panel-apply",
                Some("secret"),
                json!({"confirm": true}),
            ))
            .await
            .expect("lfg blocked response");
        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["action"], "blocked_invalid_channel_type");
        assert_eq!(
            body["result"]["blocked_reason"],
            "DL_LFG_PANEL_CHANNEL_ID zeigt auf forum-Kanal; Panel-Ziel muss ein Text-Kanal sein"
        );
        assert_eq!(
            service
                .lfg_panel_apply_calls
                .lock()
                .expect("lfg panel apply calls")
                .as_slice(),
            &[true]
        );
    }

    #[tokio::test]
    async fn onboarding_apply_http_parst_confirm_und_hash() {
        let service = Arc::new(MockServerSync::default());
        let app = router(service.clone(), Some("secret".to_string()));
        let response = app
            .oneshot(request(
                "/serversync/onboarding-apply",
                Some("secret"),
                json!({"preview_id": 21, "hash": "onboarding-hash", "confirm": true}),
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["apply_run_id"], 22);
        assert_eq!(
            service
                .onboarding_apply_calls
                .lock()
                .expect("onboarding apply calls")
                .as_slice(),
            &[(21, "onboarding-hash".to_string(), true, None)]
        );
    }

    #[tokio::test]
    async fn slash_command_ist_owner_und_guild_gegated() {
        let service = Arc::new(MockServerSync::default());
        let handler = ServerSyncCommand {
            service,
            owner_id: Some(10),
        };

        let denied = dl_discord::InteractionHandler::handle(
            &handler,
            BridgeInteraction {
                command: "serversync snapshot".to_string(),
                guild_id: GUILD_ID,
                user_id: 11,
                ..BridgeInteraction::default()
            },
        )
        .await;
        assert_eq!(denied.content.as_deref(), Some("Keine Berechtigung."));

        let wrong_guild = dl_discord::InteractionHandler::handle(
            &handler,
            BridgeInteraction {
                command: "serversync snapshot".to_string(),
                guild_id: 1,
                user_id: 10,
                ..BridgeInteraction::default()
            },
        )
        .await;
        assert_eq!(wrong_guild.content.as_deref(), Some("Keine Berechtigung."));
    }

    #[tokio::test]
    async fn slash_snapshot_owner_bekommt_report() {
        let service = Arc::new(MockServerSync::default());
        let handler = ServerSyncCommand {
            service,
            owner_id: Some(10),
        };

        let reply = dl_discord::InteractionHandler::handle(
            &handler,
            BridgeInteraction {
                command: "serversync snapshot".to_string(),
                guild_id: GUILD_ID,
                user_id: 10,
                ..BridgeInteraction::default()
            },
        )
        .await;

        assert!(reply.ephemeral);
        assert!(reply
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("snapshot_id: 10"));
    }

    #[tokio::test]
    async fn slash_onboarding_preview_owner_bekommt_hash() {
        let service = Arc::new(MockServerSync::default());
        let handler = ServerSyncCommand {
            service,
            owner_id: Some(10),
        };

        let reply = dl_discord::InteractionHandler::handle(
            &handler,
            BridgeInteraction {
                command: "serversync onboarding-preview".to_string(),
                guild_id: GUILD_ID,
                user_id: 10,
                ..BridgeInteraction::default()
            },
        )
        .await;

        assert!(reply.ephemeral);
        assert!(reply
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("diff_hash: onboarding-hash"));
    }

    #[tokio::test]
    async fn slash_archive_enable_owner_toggelt_flag() {
        let service = Arc::new(MockServerSync::default());
        let handler = ServerSyncCommand {
            service: service.clone(),
            owner_id: Some(10),
        };

        let reply = dl_discord::InteractionHandler::handle(
            &handler,
            BridgeInteraction {
                command: "serversync archive-enable".to_string(),
                guild_id: GUILD_ID,
                user_id: 10,
                ..BridgeInteraction::default()
            },
        )
        .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some("archive_enabled: true"));
        assert_eq!(
            service
                .archive_calls
                .lock()
                .expect("archive calls")
                .as_slice(),
            &[true]
        );
    }
}
