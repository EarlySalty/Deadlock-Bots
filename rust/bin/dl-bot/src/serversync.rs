#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
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

const DEFAULT_ONBOARDING_CHANNEL_NAMES: &[&str] = &[
    "allgemein",
    "frag-die-community",
    "spieler-suche",
    "memes",
    "rank-ups",
    "patchnotes",
    "deadlock-rang",
    "deadlock-invite",
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
        Self::internal(value.to_string())
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
}

pub struct ServerSyncService {
    pool: PgPool,
    adapter: Arc<dl_discord::DiscordAdapter>,
    discord_token: String,
    guild_id: u64,
    http_client: reqwest::Client,
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
        })
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
        let derivation = dl_server_as_code::derive_desired_model(&model)?;
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
        let derivation = dl_server_as_code::derive_desired_model(&live)?;
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
        validate_default_channels_7_5(&live, &preview.config.default_channel_ids)
            .map_err(ServerSyncError::bad_request)?;
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

fn build_welle2b_onboarding_config(
    live_config: &Value,
    model: &GuildModel,
) -> ServerSyncResult<OnboardingBuildOutput> {
    let live = parse_live_onboarding_config(live_config)?;
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    let default_channel_ids = resolve_default_channel_ids(model, &mut blockers);
    if !default_channel_ids.is_empty() {
        if let Err(err) = validate_default_channels_7_5(model, &default_channel_ids) {
            blockers.push(err);
        }
    }

    let spieler_suche = resolve_channel_id(model, "spieler-suche");
    let invite_role = require_role_id(model, &["Invite-Gast"], "Invite-Gast", &mut blockers);
    let frischling_role = require_role_id(model, &["Frischling"], "Frischling", &mut blockers);

    let rank_prompt = find_rank_prompt(&live, model);
    if rank_prompt.is_none() {
        blockers.push(
            "Rang-Prompt mit 12 Optionen auf `(unverifiziert)`-Rollen wurde in der Live-Onboarding-Config nicht gefunden"
                .to_string(),
        );
    }

    let mut prompts = vec![weiche_prompt(invite_role, frischling_role, spieler_suche)];
    prompts.push(ping_prompt(model, &mut blockers));
    if let Some(prompt) = rank_prompt {
        prompts.push(prompt);
    }

    if spieler_suche.is_some() {
        warnings.push(
            "`Ich spiele Deadlock und suche Mitspieler` nutzt `spieler-suche` als channel_id-Fallback, damit Discord Optionen ohne Rollen/Kanaele sicher akzeptiert."
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
    NativeOnboardingPutConfig {
        prompts: config
            .prompts
            .iter()
            .map(native_onboarding_put_prompt)
            .collect(),
        default_channel_ids: config.default_channel_ids.clone(),
        enabled: config.enabled,
        mode: config.mode.clone(),
    }
}

fn native_onboarding_put_prompt(prompt: &NativeOnboardingPrompt) -> NativeOnboardingPutPrompt {
    NativeOnboardingPutPrompt {
        id: prompt.id.clone(),
        prompt_type: prompt.prompt_type,
        title: prompt.title.clone(),
        options: prompt
            .options
            .iter()
            .map(native_onboarding_put_option)
            .collect(),
        single_select: prompt.single_select,
        required: prompt.required,
        in_onboarding: prompt.in_onboarding,
        extra: put_extra_without_emoji_fields(&prompt.extra),
    }
}

fn native_onboarding_put_option(option: &NativeOnboardingOption) -> NativeOnboardingPutOption {
    let (emoji_id, emoji_name, emoji_animated) = put_emoji_fields(option.emoji.as_ref());
    NativeOnboardingPutOption {
        id: option.id.clone(),
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
                // den ohnehin vorgesehenen Default-Kanal `spieler-suche`.
                channel_ids: spieler_suche
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default(),
                extra: BTreeMap::new(),
            },
            NativeOnboardingOption {
                id: None,
                title: "Ich hab Deadlock noch nicht — ich brauche einen Invite".to_string(),
                description: None,
                emoji: Some(json!({ "name": "🔑" })),
                role_ids: invite_role
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default(),
                channel_ids: Vec::new(),
                extra: BTreeMap::new(),
            },
            NativeOnboardingOption {
                id: None,
                title: "Ich bin ganz neu und will's lernen — nehmt mich an die Hand".to_string(),
                description: None,
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
            ],
        ),
        ("Custom Games", &["Custom Games Ping Rolle", "Custom Games"]),
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
    DEFAULT_ONBOARDING_CHANNEL_NAMES
        .iter()
        .filter_map(|name| {
            let id = resolve_channel_id(model, name);
            if id.is_none() {
                blockers.push(format!(
                    "Kanal `{name}` wurde im Live-Guild-Modell nicht gefunden"
                ));
            }
            id.map(|id| id.to_string())
        })
        .collect()
}

fn resolve_channel_id(model: &GuildModel, name: &str) -> Option<u64> {
    let expected = normalized_name(name);
    model
        .channels
        .values()
        .find(|channel| normalized_name(&channel.name) == expected)
        .map(|channel| channel.channel_id)
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
    permissions.contains(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES)
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

fn onboarding_blocker_summary(blockers: &[String]) -> String {
    format!(
        "Onboarding-Preview blockiert: {} Blocker.\n- {}",
        blockers.len(),
        blockers.join("\n- ")
    )
}

struct StoredOnboardingPreview {
    guild_id: u64,
    diff_hash: String,
    config: NativeOnboardingConfig,
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
    let (preview_id, created_snapshot_id): (i64, Option<i64>) = sqlx::query_as(
        "INSERT INTO server_config.diff_previews
         (guild_id, snapshot_id, diff_hash, diff_json, human_summary, created_by_user_id)
         VALUES ($1, $2, $3, $4::text::jsonb, $5, $6)
         ON CONFLICT (guild_id, diff_hash)
         DO UPDATE SET snapshot_id = EXCLUDED.snapshot_id,
                       diff_json = EXCLUDED.diff_json,
                       human_summary = EXCLUDED.human_summary
         RETURNING preview_id, snapshot_id",
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
        "SELECT guild_id, diff_hash, diff_json::text AS diff_json
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
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ServerSyncError::bad_request(format!(
            "Onboarding-Preview {preview_id} nicht gefunden, guild-fremd, bereits angewendet oder keine Native-Onboarding-Preview"
        ))
    })?;

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

fn is_native_onboarding_change(change: &DiffChange, expected_guild_id: u64) -> bool {
    change.object.kind == ObjectKind::BotMessage
        && change.object.guild_id == expected_guild_id
        && change.object.object_id == ONBOARDING_DIFF_OBJECT_ID
        && change.object.message_key.as_deref() == Some(ONBOARDING_DIFF_MESSAGE_KEY)
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
    router.on_command("serversync onboarding-apply", spec.clone(), handler.clone());
    router.on_command("serversync apply", spec, handler);
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
            "serversync onboarding-apply" => {
                command_onboarding_apply(self.service.as_ref(), &interaction).await
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
        .route("/serversync/onboarding-apply", post(http_onboarding_apply))
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
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::http::Request;
    use dl_server_as_code::{
        BotMessageSpec, CategorySpec, ChannelKind, ChannelSpec, OverwriteKey, RoleSpec, TargetKind,
    };
    use serenity::all::Permissions;
    use tower::ServiceExt;

    use super::*;

    type ApplyCall = (i64, String, bool, Option<u64>);
    type OnboardingApplyCall = (i64, String, bool, Option<u64>);
    type RestoreCall = (Option<i64>, Option<u64>);

    #[derive(Default)]
    struct MockServerSync {
        apply_calls: Mutex<Vec<ApplyCall>>,
        onboarding_apply_calls: Mutex<Vec<OnboardingApplyCall>>,
        restore_calls: Mutex<Vec<RestoreCall>>,
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
            (6003, "spieler-suche"),
            (6004, "memes"),
            (6005, "rank-ups"),
            (6006, "patchnotes"),
            (6007, "deadlock-rang"),
            (6008, "deadlock-invite"),
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
            "default_channel_ids": ["1"],
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

    #[test]
    fn onboarding_builder_baut_drei_prompts_und_uebernimmt_rank_prompt_unveraendert() {
        let model = onboarding_model();
        let live = rank_live_onboarding_config();

        let built = build_welle2b_onboarding_config(&live, &model).expect("build");

        assert!(built.blockers.is_empty(), "blockers: {:?}", built.blockers);
        assert!(built.config.enabled);
        assert_eq!(built.config.mode, json!(1));
        assert_eq!(built.config.default_channel_ids.len(), 9);
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
        assert!(!prompts[0]
            .as_object()
            .expect("weiche prompt")
            .contains_key("id"));
        assert!(!prompts[1]
            .as_object()
            .expect("ping prompt")
            .contains_key("id"));

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
            "UPDATE server_config.diff_previews SET applied_at = now() WHERE preview_id = $1",
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
        assert!(built
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Kanal `server-support`")));
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
}
