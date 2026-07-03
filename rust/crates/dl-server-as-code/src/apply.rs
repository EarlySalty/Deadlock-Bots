use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use serenity::all::{ChannelId, GuildId, Http, Permissions, RoleId, TargetId};
use serenity::http::StatusCode;
use sqlx::PgPool;

use crate::db;
use crate::diff::{DiffAction, DiffChange};
use crate::model::{
    BotMessageSpec, CategorySpec, ChannelSpec, ObjectKind, ObjectRef, PermissionOverwriteSpec,
    RoleSpec, TargetKind,
};
use crate::{id_to_i64, Result, ServerAsCodeError};

const STATUS_APPLIED: &str = "applied";
const STATUS_DRY_RUN: &str = "dry_run";
const STATUS_SKIPPED_MANUAL_ARCHIVE: &str =
    "skipped: manueller Schritt / Archivierung erforderlich";
const STATUS_SKIPPED_NOT_IMPLEMENTED: &str = "skipped: not implemented";
const STATUS_SKIPPED_TARGET_GONE: &str = "skipped: Objekt weg — übersprungen";
const STATUS_SKIPPED_ONBOARDING_REF: &str =
    "skipped: Kanal in Onboarding/Server-Guide referenziert — Discord verweigert Sichtbarkeits-Entzug (Code 350003)";
const DISCORD_ONBOARDING_READABLE_CODE: isize = 350_003;
const PREVIEW_MAX_AGE_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOptions {
    pub dry_run: bool,
    pub requested_by_user_id: Option<u64>,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            dry_run: true,
            requested_by_user_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub apply_run_id: i64,
    pub preview_id: i64,
    pub dry_run: bool,
    pub applied_changes: usize,
    pub change_results: Vec<ApplyChangeResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplyChangeResult {
    pub object: ObjectRef,
    pub action: DiffAction,
    pub status: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ApplyChangeOutcome {
    pub created_id: Option<u64>,
}

#[async_trait]
pub(crate) trait DiscordApplyPort {
    async fn apply_change(&self, change: &DiffChange) -> Result<ApplyChangeOutcome>;
}

pub(crate) struct SerenityApplyPort<'a> {
    pub http: &'a Http,
    pub audit_log_reason: &'a str,
}

#[async_trait]
impl DiscordApplyPort for SerenityApplyPort<'_> {
    async fn apply_change(&self, change: &DiffChange) -> Result<ApplyChangeOutcome> {
        match change.object.kind {
            ObjectKind::Category => self.apply_category(change).await,
            ObjectKind::Channel => self.apply_channel(change).await,
            ObjectKind::Role => self.apply_role(change).await,
            ObjectKind::PermissionOverwrite => self.apply_overwrite(change).await,
            ObjectKind::BotMessage => Ok(ApplyChangeOutcome::default()),
        }
    }
}

impl SerenityApplyPort<'_> {
    async fn apply_category(&self, change: &DiffChange) -> Result<ApplyChangeOutcome> {
        match change.action {
            DiffAction::Create => {
                let spec: CategorySpec = value_to_spec(change.desired.as_ref())?;
                let created = self
                    .http
                    .create_channel(
                        GuildId::new(spec.guild_id),
                        &json!({
                            "name": spec.name,
                            "type": 4,
                            "position": spec.position,
                        }),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome {
                    created_id: Some(created.id.get()),
                })
            }
            DiffAction::Update => {
                let spec: CategorySpec = value_to_spec(change.desired.as_ref())?;
                self.http
                    .edit_channel(
                        ChannelId::new(spec.category_id),
                        &json!({
                            "name": spec.name,
                            "position": spec.position,
                        }),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome::default())
            }
            DiffAction::Delete => Ok(ApplyChangeOutcome::default()),
        }
    }

    async fn apply_channel(&self, change: &DiffChange) -> Result<ApplyChangeOutcome> {
        match change.action {
            DiffAction::Create => {
                let spec: ChannelSpec = value_to_spec(change.desired.as_ref())?;
                let created = self
                    .http
                    .create_channel(
                        GuildId::new(spec.guild_id),
                        &channel_payload(&spec, true),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome {
                    created_id: Some(created.id.get()),
                })
            }
            DiffAction::Update => {
                let spec: ChannelSpec = value_to_spec(change.desired.as_ref())?;
                self.http
                    .edit_channel(
                        ChannelId::new(spec.channel_id),
                        &channel_payload(&spec, false),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome::default())
            }
            DiffAction::Delete => Ok(ApplyChangeOutcome::default()),
        }
    }

    async fn apply_role(&self, change: &DiffChange) -> Result<ApplyChangeOutcome> {
        match change.action {
            DiffAction::Create => {
                let spec: RoleSpec = value_to_spec(change.desired.as_ref())?;
                let created = self
                    .http
                    .create_role(
                        GuildId::new(spec.guild_id),
                        &role_payload(&spec),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome {
                    created_id: Some(created.id.get()),
                })
            }
            DiffAction::Update => {
                let spec: RoleSpec = value_to_spec(change.desired.as_ref())?;
                self.http
                    .edit_role(
                        GuildId::new(spec.guild_id),
                        RoleId::new(spec.role_id),
                        &role_payload(&spec),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome::default())
            }
            DiffAction::Delete => Ok(ApplyChangeOutcome::default()),
        }
    }

    async fn apply_overwrite(&self, change: &DiffChange) -> Result<ApplyChangeOutcome> {
        match change.action {
            DiffAction::Create | DiffAction::Update => {
                let spec: PermissionOverwriteSpec = value_to_spec(change.desired.as_ref())?;
                self.http
                    .create_permission(
                        ChannelId::new(spec.key.channel_id),
                        TargetId::new(spec.key.target_id),
                        &overwrite_payload(&spec),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome::default())
            }
            DiffAction::Delete => {
                let Some(target_id) = change.object.target_id else {
                    return Ok(ApplyChangeOutcome::default());
                };
                self.http
                    .delete_permission(
                        ChannelId::new(change.object.object_id),
                        TargetId::new(target_id),
                        Some(self.audit_log_reason),
                    )
                    .await?;
                Ok(ApplyChangeOutcome::default())
            }
        }
    }
}

/// Wendet eine gespeicherte Preview nur nach Confirm-Hash-Abgleich an.
///
/// `ApplyOptions::default()` ist ein Dry-Run. Live-Discord-Schreibzugriffe sind
/// nur ueber diese Funktion erreichbar; der interne Serenity-Port ist bewusst
/// nicht Teil der oeffentlichen API.
pub async fn apply_preview(
    pool: &PgPool,
    preview_id: i64,
    confirmed_diff_hash: &str,
    options: ApplyOptions,
    http: &Http,
    audit_log_reason: &str,
) -> Result<ApplyReport> {
    let port = SerenityApplyPort {
        http,
        audit_log_reason,
    };
    apply_preview_with_port(pool, preview_id, confirmed_diff_hash, options, &port).await
}

pub(crate) async fn apply_preview_with_port<P: DiscordApplyPort + Sync>(
    pool: &PgPool,
    preview_id: i64,
    confirmed_diff_hash: &str,
    options: ApplyOptions,
    port: &P,
) -> Result<ApplyReport> {
    let preview = db::load_preview(pool, preview_id).await?;
    ensure_preview_fresh(db::preview_is_expired(pool, preview_id, PREVIEW_MAX_AGE_MINUTES).await?)?;
    let recomputed_diff_hash = db::diff_hash(&preview.diff)?;
    let hashes_match =
        preview.diff_hash == recomputed_diff_hash && recomputed_diff_hash == confirmed_diff_hash;
    let initial_status = if hashes_match {
        "running"
    } else {
        "hash_mismatch"
    };
    let apply_run_id = insert_apply_run(
        pool,
        preview_id,
        &preview,
        confirmed_diff_hash,
        &options,
        initial_status,
    )
    .await?;

    if !hashes_match {
        finish_apply_run(
            pool,
            apply_run_id,
            "hash_mismatch",
            json!({
                "stored": preview.diff_hash,
                "recomputed": recomputed_diff_hash,
                "confirmed": confirmed_diff_hash,
            }),
            Some("diff hash binding mismatch"),
        )
        .await?;
        return Err(ServerAsCodeError::DiffHashMismatch {
            expected: preview.diff_hash,
            actual: confirmed_diff_hash.to_string(),
        });
    }

    let ordered_changes = apply_ordered_changes(&preview.diff.changes);
    let dry_run_results = ordered_changes
        .iter()
        .map(|change| change_result(change, STATUS_DRY_RUN))
        .collect::<Vec<_>>();

    if options.dry_run {
        finish_apply_run(
            pool,
            apply_run_id,
            "dry_run",
            json!({
                "planned_changes": preview.diff.changes.len(),
                "change_results": &dry_run_results,
            }),
            None,
        )
        .await?;
        return Ok(ApplyReport {
            apply_run_id,
            preview_id,
            dry_run: true,
            applied_changes: 0,
            change_results: dry_run_results,
        });
    }

    let mut applied = 0usize;
    let mut change_results = Vec::new();
    let mut created_ids = CreatedIdMap::default();
    for change in ordered_changes {
        if let Some(status) = skip_status(change) {
            change_results.push(change_result(change, status));
            continue;
        }

        let effective_change = remap_created_ids(change, &created_ids)?;
        let outcome = match port.apply_change(&effective_change).await {
            Ok(outcome) => outcome,
            Err(err) if is_target_gone(&err) => {
                change_results.push(change_result(change, STATUS_SKIPPED_TARGET_GONE));
                continue;
            }
            Err(err) if is_onboarding_readable_refusal(&err) => {
                change_results.push(change_result(change, STATUS_SKIPPED_ONBOARDING_REF));
                continue;
            }
            Err(err) => {
                finish_apply_run(
                    pool,
                    apply_run_id,
                    "failed",
                    json!({
                        "applied_changes": applied,
                        "change_results": change_results,
                    }),
                    Some("Apply-Lauf abgebrochen: Discord-Änderung fehlgeschlagen — Details stehen in change_results"),
                )
                .await?;
                return Err(err);
            }
        };

        if let Some(created_id) = outcome.created_id {
            if let Err(err) = db::record_created_object_id(pool, change, created_id).await {
                change_results.push(change_result(change, "failed: id mapping"));
                finish_apply_run(
                    pool,
                    apply_run_id,
                    "failed",
                    json!({
                        "applied_changes": applied,
                        "change_results": change_results,
                    }),
                    Some("created Discord id mapping failed"),
                )
                .await?;
                return Err(err);
            }
            created_ids.insert(change.object.kind, change.object.object_id, created_id);
        }

        applied += 1;
        change_results.push(change_result(change, STATUS_APPLIED));
    }

    finish_apply_run(
        pool,
        apply_run_id,
        STATUS_APPLIED,
        json!({
            "applied_changes": applied,
            "change_results": &change_results,
        }),
        None,
    )
    .await?;
    sqlx::query("UPDATE server_config.diff_previews SET applied_at = now() WHERE preview_id = $1")
        .bind(preview_id)
        .execute(pool)
        .await?;

    Ok(ApplyReport {
        apply_run_id,
        preview_id,
        dry_run: false,
        applied_changes: applied,
        change_results,
    })
}

fn ensure_preview_fresh(preview_expired: bool) -> Result<()> {
    if preview_expired {
        return Err(ServerAsCodeError::PreviewExpired);
    }
    Ok(())
}

fn apply_ordered_changes(changes: &[DiffChange]) -> Vec<&DiffChange> {
    let mut indexed = changes.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, change)| (apply_priority(change), *index));
    indexed.into_iter().map(|(_, change)| change).collect()
}

fn apply_priority(change: &DiffChange) -> u8 {
    match (change.object.kind, change.action) {
        (ObjectKind::Category | ObjectKind::Channel | ObjectKind::Role, DiffAction::Create) => 0,
        (ObjectKind::PermissionOverwrite, DiffAction::Create | DiffAction::Update)
            if overwrite_denies_view(change) =>
        {
            1
        }
        (ObjectKind::PermissionOverwrite, DiffAction::Delete) => 3,
        _ => 2,
    }
}

fn overwrite_denies_view(change: &DiffChange) -> bool {
    change
        .desired
        .as_ref()
        .and_then(|desired| serde_json::from_value::<PermissionOverwriteSpec>(desired.clone()).ok())
        .is_some_and(|overwrite| {
            Permissions::from_bits_truncate(overwrite.deny_bits).contains(Permissions::VIEW_CHANNEL)
        })
}

/// Discord lehnt Sichtbarkeits-Entzug auf Kanälen ab, die Onboarding ODER
/// Server Guide referenzieren (Code 350003). Das ist deterministisch, kein
/// transienter Fehler — der Change wird als skipped protokolliert, damit der
/// restliche Apply-Lauf nicht abbricht (analog skip-on-404, §0).
fn is_onboarding_readable_refusal(error: &ServerAsCodeError) -> bool {
    let ServerAsCodeError::Serenity(source) = error else {
        return false;
    };
    matches!(
        source.as_ref(),
        serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp))
            if resp.error.code == DISCORD_ONBOARDING_READABLE_CODE
    )
}

fn is_target_gone(error: &ServerAsCodeError) -> bool {
    match error {
        ServerAsCodeError::TargetGone { .. } => true,
        ServerAsCodeError::Serenity(source) => match source.as_ref() {
            serenity::Error::Http(http_error) => matches!(
                http_error.status_code(),
                Some(StatusCode::NOT_FOUND | StatusCode::GONE)
            ),
            _ => false,
        },
        _ => false,
    }
}

/// Entscheidet, welche Änderungen Apply bewusst NICHT ausführt.
///
/// Politik (Konzept §5.1/Prinzip 9 „archivieren statt löschen"):
/// - Kategorien/Kanäle/Rollen werden NIE automatisch gelöscht — das bleibt
///   ein manueller, dokumentierter Schritt (Archivierung).
/// - Permission-OVERWRITES dürfen dagegen gelöscht werden: Das ist der Kern
///   der Rechte-Sanierung (Welle 2a, 519→~130 Overwrites) und zerstört keine
///   Inhalte. Schutz davor liegt im Preview→Confirm-Hash→Apply-Gate mit
///   Dry-Run-Default, nicht in einem Skip.
fn skip_status(change: &DiffChange) -> Option<&'static str> {
    match (change.object.kind, change.action) {
        (ObjectKind::BotMessage, _) => Some(STATUS_SKIPPED_NOT_IMPLEMENTED),
        (ObjectKind::Category | ObjectKind::Channel | ObjectKind::Role, DiffAction::Delete) => {
            Some(STATUS_SKIPPED_MANUAL_ARCHIVE)
        }
        _ => None,
    }
}

fn change_result(change: &DiffChange, status: &str) -> ApplyChangeResult {
    ApplyChangeResult {
        object: change.object.clone(),
        action: change.action,
        status: status.to_string(),
    }
}

#[derive(Default)]
struct CreatedIdMap {
    categories: BTreeMap<u64, u64>,
    channels: BTreeMap<u64, u64>,
    roles: BTreeMap<u64, u64>,
}

impl CreatedIdMap {
    fn insert(&mut self, kind: ObjectKind, logical_id: u64, discord_id: u64) {
        match kind {
            ObjectKind::Category => {
                self.categories.insert(logical_id, discord_id);
            }
            ObjectKind::Channel => {
                self.channels.insert(logical_id, discord_id);
            }
            ObjectKind::Role => {
                self.roles.insert(logical_id, discord_id);
            }
            ObjectKind::PermissionOverwrite | ObjectKind::BotMessage => {}
        }
    }

    fn category(&self, id: u64) -> u64 {
        self.categories.get(&id).copied().unwrap_or(id)
    }

    fn channel(&self, id: u64) -> u64 {
        self.channels.get(&id).copied().unwrap_or(id)
    }

    fn role(&self, id: u64) -> u64 {
        self.roles.get(&id).copied().unwrap_or(id)
    }
}

fn remap_created_ids(change: &DiffChange, created_ids: &CreatedIdMap) -> Result<DiffChange> {
    let mut mapped = change.clone();
    mapped.object = remap_object_ref(&mapped.object, created_ids);
    mapped.desired = change
        .desired
        .as_ref()
        .map(|desired| remap_desired_value(change.object.kind, desired, created_ids))
        .transpose()?;
    Ok(mapped)
}

fn remap_object_ref(object: &ObjectRef, created_ids: &CreatedIdMap) -> ObjectRef {
    let mut mapped = object.clone();
    match mapped.kind {
        ObjectKind::Category => {
            mapped.object_id = created_ids.category(mapped.object_id);
        }
        ObjectKind::Channel => {
            mapped.object_id = created_ids.channel(mapped.object_id);
            mapped.channel_id = mapped.channel_id.map(|id| created_ids.channel(id));
        }
        ObjectKind::Role => {
            mapped.object_id = created_ids.role(mapped.object_id);
        }
        ObjectKind::PermissionOverwrite => {
            mapped.object_id = created_ids.channel(mapped.object_id);
            mapped.channel_id = mapped.channel_id.map(|id| created_ids.channel(id));
            if mapped.target_kind == Some(TargetKind::Role) {
                mapped.target_id = mapped.target_id.map(|id| created_ids.role(id));
            }
        }
        ObjectKind::BotMessage => {
            mapped.object_id = created_ids.channel(mapped.object_id);
            mapped.channel_id = mapped.channel_id.map(|id| created_ids.channel(id));
        }
    }
    mapped
}

fn remap_desired_value(
    kind: ObjectKind,
    desired: &Value,
    created_ids: &CreatedIdMap,
) -> Result<Value> {
    match kind {
        ObjectKind::Category => {
            let mut spec: CategorySpec = serde_json::from_value(desired.clone())?;
            spec.category_id = created_ids.category(spec.category_id);
            Ok(serde_json::to_value(spec)?)
        }
        ObjectKind::Channel => {
            let mut spec: ChannelSpec = serde_json::from_value(desired.clone())?;
            spec.channel_id = created_ids.channel(spec.channel_id);
            spec.parent_category_id = spec.parent_category_id.map(|id| created_ids.category(id));
            Ok(serde_json::to_value(spec)?)
        }
        ObjectKind::Role => {
            let mut spec: RoleSpec = serde_json::from_value(desired.clone())?;
            spec.role_id = created_ids.role(spec.role_id);
            Ok(serde_json::to_value(spec)?)
        }
        ObjectKind::PermissionOverwrite => {
            let mut spec: PermissionOverwriteSpec = serde_json::from_value(desired.clone())?;
            spec.key.channel_id = created_ids.channel(spec.key.channel_id);
            if spec.key.target_kind == TargetKind::Role {
                spec.key.target_id = created_ids.role(spec.key.target_id);
            }
            Ok(serde_json::to_value(spec)?)
        }
        ObjectKind::BotMessage => {
            let mut spec: BotMessageSpec = serde_json::from_value(desired.clone())?;
            spec.channel_id = created_ids.channel(spec.channel_id);
            Ok(serde_json::to_value(spec)?)
        }
    }
}

async fn insert_apply_run(
    pool: &PgPool,
    preview_id: i64,
    preview: &db::DiffPreview,
    confirmed_diff_hash: &str,
    options: &ApplyOptions,
    status: &str,
) -> Result<i64> {
    let apply_run_id = sqlx::query_scalar(
        "INSERT INTO server_config.apply_runs
         (preview_id, guild_id, confirmed_diff_hash, requested_by_user_id, dry_run, status)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING apply_run_id",
    )
    .bind(preview_id)
    .bind(id_to_i64(preview.guild_id)?)
    .bind(confirmed_diff_hash)
    .bind(options.requested_by_user_id.map(id_to_i64).transpose()?)
    .bind(options.dry_run)
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok(apply_run_id)
}

async fn finish_apply_run(
    pool: &PgPool,
    apply_run_id: i64,
    status: &str,
    result_json: Value,
    error_text: Option<&str>,
) -> Result<()> {
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
    .bind(result_json.to_string())
    .bind(error_text)
    .execute(pool)
    .await?;
    Ok(())
}

fn value_to_spec<T>(value: Option<&Value>) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let value = value.cloned().unwrap_or(Value::Null);
    Ok(serde_json::from_value(value)?)
}

fn channel_payload(spec: &ChannelSpec, include_type: bool) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("name".to_string(), json!(spec.name));
    payload.insert("topic".to_string(), json!(spec.topic));
    payload.insert("position".to_string(), json!(spec.position));
    payload.insert(
        "parent_id".to_string(),
        spec.parent_category_id
            .map(|id| json!(id.to_string()))
            .unwrap_or(Value::Null),
    );
    payload.insert("nsfw".to_string(), json!(spec.nsfw));
    payload.insert("bitrate".to_string(), json!(spec.bitrate));
    payload.insert("user_limit".to_string(), json!(spec.user_limit));
    payload.insert(
        "rate_limit_per_user".to_string(),
        json!(spec.rate_limit_per_user),
    );
    payload.insert("status".to_string(), json!(spec.status));
    if include_type {
        payload.insert("type".to_string(), json!(spec.kind.discord_type_code()));
    }
    Value::Object(payload)
}

fn role_payload(spec: &RoleSpec) -> Value {
    json!({
        "name": spec.name,
        "color": spec.color,
        "hoist": spec.hoist,
        "mentionable": spec.mentionable,
        "permissions": spec.permissions_bitmask.to_string(),
        "position": spec.position,
    })
}

fn overwrite_payload(spec: &PermissionOverwriteSpec) -> Value {
    let overwrite_type = match spec.key.target_kind {
        TargetKind::Role => 0,
        TargetKind::Member => 1,
    };
    json!({
        "id": spec.key.target_id.to_string(),
        "type": overwrite_type,
        "allow": spec.allow_bits.to_string(),
        "deny": spec.deny_bits.to_string(),
    })
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::model::{OverwriteKey, RoleSpec};

    const GUILD_ID: u64 = 1_289_721_245_281_292_288;
    const CHANNEL_ID: u64 = 10;
    const ROLE_ID: u64 = 20;

    fn role_create_change() -> DiffChange {
        let spec = RoleSpec {
            guild_id: GUILD_ID,
            role_id: ROLE_ID,
            name: "Privat".to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: 0,
            position: 0,
        };
        DiffChange {
            object: ObjectRef::role(&spec),
            action: DiffAction::Create,
            fields: Vec::new(),
            desired: Some(serde_json::to_value(spec).expect("role json")),
            actual: None,
        }
    }

    fn overwrite_change(action: DiffAction, deny: Permissions) -> DiffChange {
        let spec = PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: CHANNEL_ID,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            },
            allow_bits: 0,
            deny_bits: deny.bits(),
        };
        DiffChange {
            object: ObjectRef::overwrite(&spec),
            action,
            fields: Vec::new(),
            desired: (action != DiffAction::Delete)
                .then(|| serde_json::to_value(&spec).expect("overwrite desired")),
            actual: (action == DiffAction::Delete)
                .then(|| serde_json::to_value(&spec).expect("overwrite actual")),
        }
    }

    #[test]
    fn apply_order_sortiert_sichtbarkeits_denies_vor_overwrite_deletes() {
        let delete = overwrite_change(DiffAction::Delete, Permissions::empty());
        let visibility_deny = overwrite_change(DiffAction::Create, Permissions::VIEW_CHANNEL);
        let role_create = role_create_change();
        let changes = vec![delete.clone(), visibility_deny.clone(), role_create.clone()];

        let ordered = apply_ordered_changes(&changes);

        assert_eq!(ordered[0], &role_create);
        assert_eq!(ordered[1], &visibility_deny);
        assert_eq!(ordered[2], &delete);
    }

    #[test]
    fn target_gone_error_wird_als_skip_klassifiziert() {
        let err = ServerAsCodeError::TargetGone {
            object: overwrite_change(DiffAction::Delete, Permissions::empty()).object,
        };

        assert!(is_target_gone(&err));
    }

    #[tokio::test]
    async fn onboarding_readable_refusal_wird_als_skip_klassifiziert() {
        let body =
            r#"{"message": "Onboarding channels must be readable by everyone", "code": 350003}"#;
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(400)
                .body(body)
                .expect("http response"),
        );
        let error_response =
            serenity::http::ErrorResponse::from_response(response, reqwest::Method::PUT).await;
        let err = ServerAsCodeError::Serenity(Box::new(serenity::Error::Http(
            serenity::http::HttpError::UnsuccessfulRequest(error_response),
        )));

        assert!(is_onboarding_readable_refusal(&err));
        assert!(!is_target_gone(&err));

        let other = ServerAsCodeError::TargetGone {
            object: overwrite_change(DiffAction::Delete, Permissions::empty()).object,
        };
        assert!(!is_onboarding_readable_refusal(&other));
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use async_trait::async_trait;
    use dl_central_db::test_pool;

    use super::{
        apply_preview_with_port, ApplyChangeOutcome, ApplyOptions, DiscordApplyPort,
        STATUS_APPLIED, STATUS_SKIPPED_MANUAL_ARCHIVE, STATUS_SKIPPED_NOT_IMPLEMENTED,
        STATUS_SKIPPED_TARGET_GONE,
    };
    use crate::db;
    use crate::diff::diff_models;
    use crate::model::{
        BotMessageSpec, CategorySpec, ChannelKind, ChannelSpec, GuildModel, ObjectKind, ObjectRef,
        OverwriteKey, PermissionOverwriteSpec, RoleSpec, TargetKind,
    };
    use crate::{Result, DEFAULT_GUILD_ID};

    const CATEGORY_ID: u64 = 700;
    const CHANNEL_ID: u64 = 701;
    const ROLE_ID: u64 = 800;
    const ROLE_ID_2: u64 = 801;
    const ROLE_ID_3: u64 = 802;
    const CREATED_CATEGORY_ID: u64 = 17_700;
    const CREATED_CHANNEL_ID: u64 = 17_701;
    const CREATED_ROLE_ID: u64 = 17_800;

    #[derive(Default)]
    struct MappingPort {
        seen: Arc<Mutex<Vec<crate::diff::DiffChange>>>,
    }

    #[async_trait]
    impl DiscordApplyPort for MappingPort {
        async fn apply_change(
            &self,
            change: &crate::diff::DiffChange,
        ) -> Result<ApplyChangeOutcome> {
            self.seen.lock().expect("seen lock").push(change.clone());
            let created_id = match change.object.kind {
                ObjectKind::Category => Some(CREATED_CATEGORY_ID),
                ObjectKind::Channel => Some(CREATED_CHANNEL_ID),
                ObjectKind::Role => Some(CREATED_ROLE_ID),
                ObjectKind::PermissionOverwrite | ObjectKind::BotMessage => None,
            };
            Ok(ApplyChangeOutcome { created_id })
        }
    }

    #[derive(Default)]
    struct CountingPort {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DiscordApplyPort for CountingPort {
        async fn apply_change(
            &self,
            _change: &crate::diff::DiffChange,
        ) -> Result<ApplyChangeOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ApplyChangeOutcome::default())
        }
    }

    #[derive(Default)]
    struct GoneMiddlePort {
        seen: Arc<Mutex<Vec<crate::diff::DiffChange>>>,
    }

    #[async_trait]
    impl DiscordApplyPort for GoneMiddlePort {
        async fn apply_change(
            &self,
            change: &crate::diff::DiffChange,
        ) -> Result<ApplyChangeOutcome> {
            self.seen.lock().expect("seen lock").push(change.clone());
            if change.object.object_id == ROLE_ID_2 {
                return Err(crate::ServerAsCodeError::TargetGone {
                    object: change.object.clone(),
                });
            }
            Ok(ApplyChangeOutcome::default())
        }
    }

    fn category(id: u64) -> CategorySpec {
        CategorySpec {
            guild_id: DEFAULT_GUILD_ID,
            category_id: id,
            name: "Chat".to_string(),
            position: 1,
        }
    }

    fn channel(id: u64, parent: Option<u64>) -> ChannelSpec {
        ChannelSpec {
            guild_id: DEFAULT_GUILD_ID,
            channel_id: id,
            name: "allgemein".to_string(),
            kind: ChannelKind::Text,
            topic: None,
            position: 1,
            parent_category_id: parent,
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            status: None,
        }
    }

    fn role(id: u64) -> RoleSpec {
        RoleSpec {
            guild_id: DEFAULT_GUILD_ID,
            role_id: id,
            name: "Member".to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: 7,
            position: 1,
        }
    }

    fn role_create_change(id: u64) -> crate::diff::DiffChange {
        let spec = role(id);
        crate::diff::DiffChange {
            object: ObjectRef::role(&spec),
            action: crate::diff::DiffAction::Create,
            fields: Vec::new(),
            desired: Some(serde_json::to_value(spec).expect("role desired")),
            actual: None,
        }
    }

    fn overwrite(channel_id: u64, role_id: u64) -> PermissionOverwriteSpec {
        PermissionOverwriteSpec {
            guild_id: DEFAULT_GUILD_ID,
            key: OverwriteKey {
                channel_id,
                target_kind: TargetKind::Role,
                target_id: role_id,
            },
            allow_bits: 1,
            deny_bits: 0,
        }
    }

    #[tokio::test]
    #[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
    async fn create_apply_schreibt_discord_ids_ins_sollmodell_und_remappt_folgechanges(
    ) -> anyhow::Result<()> {
        let pool = test_pool().await?;
        sqlx::query(
            "INSERT INTO server_config.desired_categories (guild_id, category_id, name, position)
             VALUES ($1, $2, 'Chat', 1)",
        )
        .bind(DEFAULT_GUILD_ID as i64)
        .bind(CATEGORY_ID as i64)
        .execute(&*pool)
        .await?;
        sqlx::query(
            "INSERT INTO server_config.desired_channels
             (guild_id, channel_id, name, channel_type, topic, position, parent_category_id,
              nsfw, bitrate, user_limit, rate_limit_per_user, status)
             VALUES ($1, $2, 'allgemein', 'text', NULL, 1, $3, false, NULL, NULL, NULL, NULL)",
        )
        .bind(DEFAULT_GUILD_ID as i64)
        .bind(CHANNEL_ID as i64)
        .bind(CATEGORY_ID as i64)
        .execute(&*pool)
        .await?;
        sqlx::query(
            "INSERT INTO server_config.desired_roles
             (guild_id, role_id, name, color, hoist, mentionable, managed, permissions_bitmask, position)
             VALUES ($1, $2, 'Member', 0, false, false, false, 7, 1)",
        )
        .bind(DEFAULT_GUILD_ID as i64)
        .bind(ROLE_ID as i64)
        .execute(&*pool)
        .await?;
        sqlx::query(
            "INSERT INTO server_config.desired_permission_overwrites
             (guild_id, channel_id, target_type, target_id, allow_bits, deny_bits)
             VALUES ($1, $2, 'role', $3, 1, 0)",
        )
        .bind(DEFAULT_GUILD_ID as i64)
        .bind(CHANNEL_ID as i64)
        .bind(ROLE_ID as i64)
        .execute(&*pool)
        .await?;

        let mut desired = GuildModel::new(DEFAULT_GUILD_ID);
        desired
            .categories
            .insert(CATEGORY_ID, category(CATEGORY_ID));
        desired
            .channels
            .insert(CHANNEL_ID, channel(CHANNEL_ID, Some(CATEGORY_ID)));
        desired.roles.insert(ROLE_ID, role(ROLE_ID));
        desired.overwrites.insert(
            OverwriteKey {
                channel_id: CHANNEL_ID,
                target_kind: TargetKind::Role,
                target_id: ROLE_ID,
            },
            overwrite(CHANNEL_ID, ROLE_ID),
        );
        let actual = GuildModel::new(DEFAULT_GUILD_ID);
        let diff = diff_models(&desired, &actual, &[], &[])?;
        let preview = db::persist_diff_preview(&pool, None, &diff, None).await?;
        let port = MappingPort::default();

        let report = apply_preview_with_port(
            &pool,
            preview.preview_id,
            &preview.diff_hash,
            ApplyOptions {
                dry_run: false,
                requested_by_user_id: None,
            },
            &port,
        )
        .await?;

        assert_eq!(report.applied_changes, 4);
        {
            let seen = port.seen.lock().expect("seen lock");
            let channel_create = seen
                .iter()
                .find(|change| change.object.kind == ObjectKind::Channel)
                .expect("channel create");
            let channel_spec: ChannelSpec =
                serde_json::from_value(channel_create.desired.clone().expect("channel desired"))?;
            assert_eq!(channel_spec.parent_category_id, Some(CREATED_CATEGORY_ID));
            let overwrite_create = seen
                .iter()
                .find(|change| change.object.kind == ObjectKind::PermissionOverwrite)
                .expect("overwrite create");
            let overwrite_spec: PermissionOverwriteSpec = serde_json::from_value(
                overwrite_create.desired.clone().expect("overwrite desired"),
            )?;
            assert_eq!(overwrite_spec.key.channel_id, CREATED_CHANNEL_ID);
            assert_eq!(overwrite_spec.key.target_id, CREATED_ROLE_ID);
        }

        let loaded = db::load_desired_model(&pool, DEFAULT_GUILD_ID).await?;
        assert!(!loaded.categories.contains_key(&CATEGORY_ID));
        assert!(loaded.categories.contains_key(&CREATED_CATEGORY_ID));
        assert!(!loaded.channels.contains_key(&CHANNEL_ID));
        assert!(loaded.channels.contains_key(&CREATED_CHANNEL_ID));
        assert!(!loaded.roles.contains_key(&ROLE_ID));
        assert!(loaded.roles.contains_key(&CREATED_ROLE_ID));
        assert!(loaded.overwrites.contains_key(&OverwriteKey {
            channel_id: CREATED_CHANNEL_ID,
            target_kind: TargetKind::Role,
            target_id: CREATED_ROLE_ID,
        }));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
    async fn apply_markiert_deletes_und_botmessages_als_skip_ohne_port_call() -> anyhow::Result<()>
    {
        let pool = test_pool().await?;
        let mut desired = GuildModel::new(DEFAULT_GUILD_ID);
        desired.bot_messages.insert(
            (CHANNEL_ID, "panel-main".to_string()),
            BotMessageSpec {
                guild_id: DEFAULT_GUILD_ID,
                channel_id: CHANNEL_ID,
                message_key: "panel-main".to_string(),
                message_kind: "panel".to_string(),
                message_id: None,
                content_hash: Some("hash-a".to_string()),
            },
        );
        let mut actual = GuildModel::new(DEFAULT_GUILD_ID);
        actual
            .channels
            .insert(CHANNEL_ID, channel(CHANNEL_ID, None));
        let diff = diff_models(&desired, &actual, &[], &[])?;
        let preview = db::persist_diff_preview(&pool, None, &diff, None).await?;
        let port = CountingPort::default();

        let report = apply_preview_with_port(
            &pool,
            preview.preview_id,
            &preview.diff_hash,
            ApplyOptions {
                dry_run: false,
                requested_by_user_id: None,
            },
            &port,
        )
        .await?;

        assert_eq!(report.applied_changes, 0);
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
        assert!(report.change_results.iter().any(|result| {
            result.object.kind == ObjectKind::Channel
                && result.status == STATUS_SKIPPED_MANUAL_ARCHIVE
        }));
        assert!(report.change_results.iter().any(|result| {
            result.object
                == ObjectRef {
                    kind: ObjectKind::BotMessage,
                    guild_id: DEFAULT_GUILD_ID,
                    object_id: CHANNEL_ID,
                    channel_id: Some(CHANNEL_ID),
                    target_kind: None,
                    target_id: None,
                    message_key: Some("panel-main".to_string()),
                }
                && result.status == STATUS_SKIPPED_NOT_IMPLEMENTED
        }));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
    async fn apply_ueberspringt_404_gone_und_fuehrt_rest_weiter() -> anyhow::Result<()> {
        let pool = test_pool().await?;
        let diff = crate::diff::ServerDiff {
            guild_id: DEFAULT_GUILD_ID,
            changes: vec![
                role_create_change(ROLE_ID),
                role_create_change(ROLE_ID_2),
                role_create_change(ROLE_ID_3),
            ],
            filtered: Vec::new(),
            blocked: Vec::new(),
        };
        let preview = db::persist_diff_preview(&pool, None, &diff, None).await?;
        let port = GoneMiddlePort::default();

        let report = apply_preview_with_port(
            &pool,
            preview.preview_id,
            &preview.diff_hash,
            ApplyOptions {
                dry_run: false,
                requested_by_user_id: None,
            },
            &port,
        )
        .await?;

        assert_eq!(report.applied_changes, 2);
        assert_eq!(port.seen.lock().expect("seen lock").len(), 3);
        assert_eq!(
            report
                .change_results
                .iter()
                .filter(|result| result.status == STATUS_SKIPPED_TARGET_GONE)
                .count(),
            1
        );
        assert_eq!(report.change_results[2].status, STATUS_APPLIED);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
    async fn apply_lehnt_abgelaufene_preview_ab() -> anyhow::Result<()> {
        let pool = test_pool().await?;
        let diff = crate::diff::ServerDiff {
            guild_id: DEFAULT_GUILD_ID,
            changes: vec![role_create_change(ROLE_ID)],
            filtered: Vec::new(),
            blocked: Vec::new(),
        };
        let preview = db::persist_diff_preview(&pool, None, &diff, None).await?;
        sqlx::query(
            "UPDATE server_config.diff_previews
                SET created_at = now() - interval '16 minutes'
              WHERE preview_id = $1",
        )
        .bind(preview.preview_id)
        .execute(&*pool)
        .await?;

        let err = apply_preview_with_port(
            &pool,
            preview.preview_id,
            &preview.diff_hash,
            ApplyOptions {
                dry_run: true,
                requested_by_user_id: None,
            },
            &CountingPort::default(),
        )
        .await
        .expect_err("stale preview must fail");

        assert!(matches!(err, crate::ServerAsCodeError::PreviewExpired));
        Ok(())
    }
}
