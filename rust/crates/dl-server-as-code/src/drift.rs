use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::db::{
    load_desired_model, load_documented_exceptions, load_dynamic_namespaces, load_snapshot_model,
    persist_diff_preview, DiffPreview,
};
use crate::diff::{diff_json, diff_models, DiffChange, FieldDiff, FilterReason, FilteredDiff};
use crate::model::{DiscordId, ObjectKind, TargetKind};
use crate::{i64_to_bitmask, i64_to_id, id_to_i64, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRevertRule {
    pub whitelist_id: i64,
    pub guild_id: DiscordId,
    pub object_kind: ObjectKind,
    pub object_id: Option<DiscordId>,
    pub target_kind: Option<TargetKind>,
    pub target_id: Option<DiscordId>,
    pub permission_bits: u64,
    pub action: String,
}

#[derive(Debug, Clone)]
pub struct DriftRecord {
    pub preview: DiffPreview,
    pub drift_event_ids: Vec<i64>,
}

pub async fn detect_and_record_drift(
    pool: &PgPool,
    snapshot_id: i64,
    detected_by_user_id: Option<DiscordId>,
) -> Result<DriftRecord> {
    let actual = load_snapshot_model(pool, snapshot_id).await?;
    let desired = load_desired_model(pool, actual.guild_id).await?;
    let dynamic_namespaces = load_dynamic_namespaces(pool, actual.guild_id).await?;
    let documented_exceptions = load_documented_exceptions(pool, actual.guild_id).await?;
    let diff = diff_models(
        &desired,
        &actual,
        &dynamic_namespaces,
        &documented_exceptions,
    )?;
    let preview = persist_diff_preview(pool, Some(snapshot_id), &diff, detected_by_user_id).await?;
    let whitelist = load_auto_revert_whitelist(pool, actual.guild_id).await?;
    let mut drift_event_ids = Vec::new();
    let mut current_fingerprints = BTreeSet::new();

    for change in &diff.changes {
        for fingerprint in change_fingerprints(change) {
            current_fingerprints.insert(fingerprint.clone());
            drift_event_ids.push(
                upsert_change_event(
                    pool,
                    snapshot_id,
                    preview.preview_id,
                    change,
                    &fingerprint,
                    &whitelist,
                )
                .await?,
            );
        }
    }
    for filtered in &diff.filtered {
        for fingerprint in change_fingerprints(&filtered.change) {
            current_fingerprints.insert(fingerprint.clone());
            drift_event_ids.push(
                upsert_filtered_event(
                    pool,
                    snapshot_id,
                    preview.preview_id,
                    filtered,
                    &fingerprint,
                )
                .await?,
            );
        }
    }
    resolve_missing_events(pool, actual.guild_id, &current_fingerprints).await?;

    Ok(DriftRecord {
        preview,
        drift_event_ids,
    })
}

pub async fn load_auto_revert_whitelist(
    pool: &PgPool,
    guild_id: DiscordId,
) -> Result<Vec<AutoRevertRule>> {
    let rows = sqlx::query(
        "SELECT whitelist_id, guild_id, object_kind, object_id, target_type, target_id,
                permission_bits, action
           FROM server_config.auto_revert_whitelist
          WHERE guild_id = $1 AND active
          ORDER BY whitelist_id",
    )
    .bind(id_to_i64(guild_id)?)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let target_type: Option<String> = row.try_get("target_type")?;
            Ok(AutoRevertRule {
                whitelist_id: row.try_get("whitelist_id")?,
                guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
                object_kind: ObjectKind::from_db(
                    row.try_get::<String, _>("object_kind")?.as_str(),
                )?,
                object_id: row.try_get::<Option<i64>, _>("object_id")?.map(i64_to_id),
                target_kind: target_type
                    .as_deref()
                    .map(TargetKind::from_db)
                    .transpose()?,
                target_id: row.try_get::<Option<i64>, _>("target_id")?.map(i64_to_id),
                permission_bits: i64_to_bitmask(row.try_get("permission_bits")?),
                action: row.try_get("action")?,
            })
        })
        .collect()
}

async fn upsert_change_event(
    pool: &PgPool,
    snapshot_id: i64,
    preview_id: i64,
    change: &DiffChange,
    fingerprint: &str,
    whitelist: &[AutoRevertRule],
) -> Result<i64> {
    let auto_revert_eligible = whitelist
        .iter()
        .any(|rule| rule_matches_change(rule, change));
    let severity = if auto_revert_eligible {
        "critical"
    } else {
        "warning"
    };
    if let Some(id) = sqlx::query_scalar(
        "UPDATE server_config.drift_events
            SET snapshot_id = $2,
                preview_id = $3,
                event_kind = 'drift',
                severity = $4,
                object_kind = $5,
                object_id = $6,
                action = $7,
                diff_json = $8::text::jsonb,
                filtered_dynamic = false,
                documented_exception_id = NULL,
                auto_revert_eligible = $9,
                status = 'open',
                last_seen_at = now()
          WHERE guild_id = $1
            AND fingerprint = $10
            AND resolved_at IS NULL
          RETURNING drift_event_id",
    )
    .bind(id_to_i64(change.object.guild_id)?)
    .bind(snapshot_id)
    .bind(preview_id)
    .bind(severity)
    .bind(change.object.kind.as_db())
    .bind(id_to_i64(change.object.object_id)?)
    .bind(format!("{:?}", change.action).to_lowercase())
    .bind(serde_json::to_string(&diff_json(change))?)
    .bind(auto_revert_eligible)
    .bind(fingerprint)
    .fetch_optional(pool)
    .await?
    {
        return Ok(id);
    }

    let id = sqlx::query_scalar(
        "INSERT INTO server_config.drift_events
         (guild_id, snapshot_id, preview_id, fingerprint, event_kind, severity, object_kind, object_id,
          action, diff_json, auto_revert_eligible)
         VALUES ($1, $2, $3, $4, 'drift', $5, $6, $7, $8, $9::text::jsonb, $10)
         RETURNING drift_event_id",
    )
    .bind(id_to_i64(change.object.guild_id)?)
    .bind(snapshot_id)
    .bind(preview_id)
    .bind(fingerprint)
    .bind(severity)
    .bind(change.object.kind.as_db())
    .bind(id_to_i64(change.object.object_id)?)
    .bind(format!("{:?}", change.action).to_lowercase())
    .bind(serde_json::to_string(&diff_json(change))?)
    .bind(auto_revert_eligible)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn upsert_filtered_event(
    pool: &PgPool,
    snapshot_id: i64,
    preview_id: i64,
    filtered: &FilteredDiff,
    fingerprint: &str,
) -> Result<i64> {
    let (event_kind, documented_exception_id, filtered_dynamic) = match &filtered.reason {
        FilterReason::DynamicNamespace { .. } => ("filtered_dynamic", None, true),
        FilterReason::DocumentedException { exception_id, .. } => {
            ("documented_exception", *exception_id, false)
        }
    };
    if let Some(id) = sqlx::query_scalar(
        "UPDATE server_config.drift_events
            SET snapshot_id = $2,
                preview_id = $3,
                event_kind = $4,
                severity = 'info',
                object_kind = $5,
                object_id = $6,
                action = $7,
                diff_json = $8::text::jsonb,
                filtered_dynamic = $9,
                documented_exception_id = $10,
                auto_revert_eligible = false,
                status = 'open',
                last_seen_at = now()
          WHERE guild_id = $1
            AND fingerprint = $11
            AND resolved_at IS NULL
          RETURNING drift_event_id",
    )
    .bind(id_to_i64(filtered.change.object.guild_id)?)
    .bind(snapshot_id)
    .bind(preview_id)
    .bind(event_kind)
    .bind(filtered.change.object.kind.as_db())
    .bind(id_to_i64(filtered.change.object.object_id)?)
    .bind(format!("{:?}", filtered.change.action).to_lowercase())
    .bind(serde_json::to_string(&diff_json(&filtered.change))?)
    .bind(filtered_dynamic)
    .bind(documented_exception_id)
    .bind(fingerprint)
    .fetch_optional(pool)
    .await?
    {
        return Ok(id);
    }

    let id = sqlx::query_scalar(
        "INSERT INTO server_config.drift_events
         (guild_id, snapshot_id, preview_id, fingerprint, event_kind, severity, object_kind, object_id,
          action, diff_json, filtered_dynamic, documented_exception_id, auto_revert_eligible)
         VALUES ($1, $2, $3, $4, $5, 'info', $6, $7, $8, $9::text::jsonb, $10, $11, false)
         RETURNING drift_event_id",
    )
    .bind(id_to_i64(filtered.change.object.guild_id)?)
    .bind(snapshot_id)
    .bind(preview_id)
    .bind(fingerprint)
    .bind(event_kind)
    .bind(filtered.change.object.kind.as_db())
    .bind(id_to_i64(filtered.change.object.object_id)?)
    .bind(format!("{:?}", filtered.change.action).to_lowercase())
    .bind(serde_json::to_string(&diff_json(&filtered.change))?)
    .bind(filtered_dynamic)
    .bind(documented_exception_id)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn resolve_missing_events(
    pool: &PgPool,
    guild_id: DiscordId,
    current_fingerprints: &BTreeSet<String>,
) -> Result<()> {
    if current_fingerprints.is_empty() {
        sqlx::query(
            "UPDATE server_config.drift_events
                SET status = 'resolved',
                    resolved_at = now(),
                    last_seen_at = COALESCE(last_seen_at, detected_at)
              WHERE guild_id = $1
                AND resolved_at IS NULL",
        )
        .bind(id_to_i64(guild_id)?)
        .execute(pool)
        .await?;
        return Ok(());
    }

    let fingerprints = current_fingerprints.iter().cloned().collect::<Vec<_>>();
    sqlx::query(
        "UPDATE server_config.drift_events
            SET status = 'resolved',
                resolved_at = now(),
                last_seen_at = COALESCE(last_seen_at, detected_at)
          WHERE guild_id = $1
            AND resolved_at IS NULL
            AND NOT (fingerprint = ANY($2::text[]))",
    )
    .bind(id_to_i64(guild_id)?)
    .bind(fingerprints)
    .execute(pool)
    .await?;
    Ok(())
}

fn change_fingerprints(change: &DiffChange) -> Vec<String> {
    if change.fields.is_empty() {
        return vec![change_fingerprint(
            change,
            format!("{:?}", change.action).to_lowercase().as_str(),
        )];
    }

    change
        .fields
        .iter()
        .map(|field| change_fingerprint(change, field.field.as_str()))
        .collect()
}

fn change_fingerprint(change: &DiffChange, field_name: &str) -> String {
    let target_kind = change
        .object
        .target_kind
        .map(TargetKind::as_db)
        .unwrap_or("-");
    let target_id = change
        .object
        .target_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_string());
    let message_key = change.object.message_key.as_deref().unwrap_or("-");
    format!(
        "{}:{}:{}:{}:{}:{}",
        change.object.kind.as_db(),
        change.object.object_id,
        target_kind,
        target_id,
        message_key,
        field_name
    )
}

fn rule_matches_change(rule: &AutoRevertRule, change: &DiffChange) -> bool {
    if rule.object_kind != change.object.kind {
        return false;
    }
    if rule
        .object_id
        .is_some_and(|id| id != change.object.object_id)
    {
        return false;
    }
    if rule
        .target_kind
        .is_some_and(|target_kind| Some(target_kind) != change.object.target_kind)
    {
        return false;
    }
    if rule
        .target_id
        .is_some_and(|target_id| Some(target_id) != change.object.target_id)
    {
        return false;
    }

    match change.object.kind {
        ObjectKind::Role => role_permission_added(change, rule.permission_bits),
        ObjectKind::PermissionOverwrite => overwrite_permission_drift(change, rule),
        _ => false,
    }
}

fn role_permission_added(change: &DiffChange, permission_bits: u64) -> bool {
    let Some(field) = field(change, "permissions_bitmask") else {
        return false;
    };
    let desired = value_u64(&field.desired);
    let actual = value_u64(&field.actual);
    (actual & !desired & permission_bits) != 0
}

fn overwrite_permission_drift(change: &DiffChange, rule: &AutoRevertRule) -> bool {
    match rule.action.as_str() {
        "allow_added" => permission_delta(change, "allow_bits", rule.permission_bits, true),
        "deny_removed" => permission_delta(change, "deny_bits", rule.permission_bits, false),
        "permission_added" => {
            permission_delta(change, "allow_bits", rule.permission_bits, true)
                || permission_delta(change, "deny_bits", rule.permission_bits, false)
        }
        _ => false,
    }
}

fn permission_delta(
    change: &DiffChange,
    field_name: &str,
    permission_bits: u64,
    added: bool,
) -> bool {
    let Some(field) = field(change, field_name) else {
        return false;
    };
    let desired = value_u64(&field.desired);
    let actual = value_u64(&field.actual);
    let delta = if added {
        actual & !desired
    } else {
        desired & !actual
    };
    (delta & permission_bits) != 0
}

fn field<'a>(change: &'a DiffChange, name: &str) -> Option<&'a FieldDiff> {
    change.fields.iter().find(|field| field.field == name)
}

fn value_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
        .unwrap_or(0)
}
