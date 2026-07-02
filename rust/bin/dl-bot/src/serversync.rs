#![allow(clippy::result_large_err)]

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
use dl_discord::{BridgeAttachment, BridgeInteraction, BridgeReply, CommandSpec};
use dl_server_as_code::diff::ServerDiff;
use dl_server_as_code::{ApplyOptions, ApplyReport, SnapshotImportReport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::GuildId;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::master;

pub const GUILD_ID: u64 = dl_server_as_code::DEFAULT_GUILD_ID;
pub const PORT: u16 = 8901;
pub const TOKEN_HEADER: &str = "X-Internal-Token";
const AUDIT_LOG_REASON: &str = "Onboarding-Redesign Welle 2a (Rechte-Sanierung)";
const ROLLBACK_VERSION: &str = "serversync.rollback_export.v1";
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_DISCORD_CONTENT_CHARS: usize = 1800;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackArtifact {
    pub version: String,
    pub guild_id: u64,
    pub created_at: String,
    pub snapshot_id: i64,
    pub structure_snapshot: dl_server_as_code::GuildModel,
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
    async fn apply(
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
        let member_role_assignments = self.fetch_member_role_assignments().await?;
        let native_onboarding_config = self.fetch_native_onboarding_config().await?;
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let artifact = RollbackArtifact {
            version: ROLLBACK_VERSION.to_string(),
            guild_id: self.guild_id,
            created_at: created_at.clone(),
            snapshot_id: report.snapshot_id,
            structure_snapshot: model,
            member_role_assignments,
            native_onboarding_config,
        };
        let artifact_value = serde_json::to_value(&artifact)?;
        let artifact_text = serde_json::to_string_pretty(&artifact_value)?;
        let artifact_hash = sha256_hex(serde_json::to_vec(&artifact_value)?.as_slice());
        let metadata = json!({
            "version": ROLLBACK_VERSION,
            "member_count": artifact.member_role_assignments.len(),
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
        let preview =
            dl_server_as_code::persist_diff_preview(&self.pool, None, &diff, requested_by_user_id)
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

fn id_to_i64(value: u64) -> ServerSyncResult<i64> {
    i64::try_from(value)
        .map_err(|_| ServerSyncError::bad_request("Discord-ID passt nicht in BIGINT"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
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
        Ok(output) => BridgeReply {
            content: Some(format!(
                "Rollback-Export erstellt.\nrollback_export_id: {}\nsnapshot_id: {}\nartifact_hash: {}\nMember: {} | Rollen-Zuweisungen: {}",
                output.rollback_export_id,
                output.snapshot_id,
                output.artifact_hash,
                output.members,
                output.member_role_edges
            )),
            ephemeral: true,
            attachments: vec![BridgeAttachment {
                filename: output.filename,
                data: output.artifact_text.into_bytes(),
            }],
            ..BridgeReply::default()
        },
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
            BridgeReply {
                content: Some(truncate_discord(&text)),
                ephemeral: true,
                attachments: vec![BridgeAttachment {
                    filename: format!("serversync-diff-{}.json", output.preview_id),
                    data: output.diff_text.into_bytes(),
                }],
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
        Ok(output) => BridgeReply {
            content: Some(format!(
                "Apply-Report.\npreview_id: {}\napply_run_id: {}\nModus: {}\napplied: {} | skipped: {} | failed: {}",
                output.preview_id,
                output.apply_run_id,
                if output.dry_run { "dry-run" } else { "live" },
                output.applied,
                output.skipped,
                output.failed
            )),
            ephemeral: true,
            attachments: vec![BridgeAttachment {
                filename: format!("serversync-apply-{}.json", output.apply_run_id),
                data: output.details_text.into_bytes(),
            }],
            ..BridgeReply::default()
        },
        Err(err) => command_error(err),
    }
}

fn command_error(error: ServerSyncError) -> BridgeReply {
    BridgeReply::ephemeral_text(format!("Server-Sync fehlgeschlagen: {error}"))
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
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
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
    Json(body): Json<ApplyRequest>,
) -> Response {
    if let Err(error) = authorize(&state, &peer, &headers) {
        return json_error(error);
    }
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
    use tower::ServiceExt;

    use super::*;

    type ApplyCall = (i64, String, bool, Option<u64>);

    #[derive(Default)]
    struct MockServerSync {
        apply_calls: Mutex<Vec<ApplyCall>>,
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
    }

    fn request(path: &str, token: Option<&str>, body: Value) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header(TOKEN_HEADER, token);
        }
        let mut request = builder.body(Body::from(body.to_string())).expect("request");
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
}
