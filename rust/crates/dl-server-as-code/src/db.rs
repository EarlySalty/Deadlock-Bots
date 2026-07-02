use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

use crate::diff::{diff_json, DiffAction, DiffChange, ServerDiff};
use crate::format;
use crate::import::SnapshotImportReport;
use crate::model::{
    BotMessageSpec, CategorySpec, ChannelKind, ChannelSpec, DiscordId, DocumentedException,
    DynamicNamespace, GuildModel, NamespaceMatch, ObjectKind, OverwriteKey,
    PermissionOverwriteSpec, RoleSpec, TargetKind,
};
use crate::{bitmask_to_i64, i64_to_bitmask, i64_to_id, id_to_i64, Result, ServerAsCodeError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPreview {
    pub preview_id: i64,
    pub guild_id: DiscordId,
    pub snapshot_id: Option<i64>,
    pub diff_hash: String,
    pub human_summary: String,
    pub diff: ServerDiff,
}

pub async fn persist_snapshot_model(
    pool: &PgPool,
    model: &GuildModel,
    source: &str,
) -> Result<SnapshotImportReport> {
    let mut tx = pool.begin().await?;
    let (snapshot_id, captured_at): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO server_config.live_snapshots (guild_id, source)
         VALUES ($1, $2)
         RETURNING snapshot_id, captured_at",
    )
    .bind(id_to_i64(model.guild_id)?)
    .bind(source)
    .fetch_one(&mut *tx)
    .await?;

    for category in model.categories.values() {
        sqlx::query(
            "INSERT INTO server_config.live_snapshot_categories
             (snapshot_id, captured_at, guild_id, category_id, name, position)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(snapshot_id)
        .bind(captured_at)
        .bind(id_to_i64(category.guild_id)?)
        .bind(id_to_i64(category.category_id)?)
        .bind(&category.name)
        .bind(category.position)
        .execute(&mut *tx)
        .await?;
    }

    for channel in model.channels.values() {
        sqlx::query(
            "INSERT INTO server_config.live_snapshot_channels
             (snapshot_id, captured_at, guild_id, channel_id, name, channel_type, topic,
              position, parent_category_id, nsfw, bitrate, user_limit, rate_limit_per_user, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(snapshot_id)
        .bind(captured_at)
        .bind(id_to_i64(channel.guild_id)?)
        .bind(id_to_i64(channel.channel_id)?)
        .bind(&channel.name)
        .bind(channel.kind.as_db())
        .bind(&channel.topic)
        .bind(channel.position)
        .bind(optional_id(channel.parent_category_id)?)
        .bind(channel.nsfw)
        .bind(channel.bitrate)
        .bind(channel.user_limit)
        .bind(channel.rate_limit_per_user)
        .bind(&channel.status)
        .execute(&mut *tx)
        .await?;
    }

    for role in model.roles.values() {
        sqlx::query(
            "INSERT INTO server_config.live_snapshot_roles
             (snapshot_id, captured_at, guild_id, role_id, name, color, hoist, mentionable,
              managed, permissions_bitmask, position)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(snapshot_id)
        .bind(captured_at)
        .bind(id_to_i64(role.guild_id)?)
        .bind(id_to_i64(role.role_id)?)
        .bind(&role.name)
        .bind(role.color)
        .bind(role.hoist)
        .bind(role.mentionable)
        .bind(role.managed)
        .bind(bitmask_to_i64(role.permissions_bitmask)?)
        .bind(role.position)
        .execute(&mut *tx)
        .await?;
    }

    for overwrite in model.overwrites.values() {
        sqlx::query(
            "INSERT INTO server_config.live_snapshot_permission_overwrites
             (snapshot_id, captured_at, guild_id, channel_id, target_type, target_id, allow_bits, deny_bits)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(snapshot_id)
        .bind(captured_at)
        .bind(id_to_i64(overwrite.guild_id)?)
        .bind(id_to_i64(overwrite.key.channel_id)?)
        .bind(overwrite.key.target_kind.as_db())
        .bind(id_to_i64(overwrite.key.target_id)?)
        .bind(bitmask_to_i64(overwrite.allow_bits)?)
        .bind(bitmask_to_i64(overwrite.deny_bits)?)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(SnapshotImportReport {
        snapshot_id,
        guild_id: model.guild_id,
        captured_at,
        categories: model.categories.len(),
        channels: model.channels.len(),
        roles: model.roles.len(),
        overwrites: model.overwrites.len(),
    })
}

pub async fn load_desired_model(pool: &PgPool, guild_id: DiscordId) -> Result<GuildModel> {
    let mut model = GuildModel::new(guild_id);
    let guild_i64 = id_to_i64(guild_id)?;

    for row in sqlx::query(
        "SELECT guild_id, category_id, name, position
           FROM server_config.desired_categories
          WHERE guild_id = $1
          ORDER BY category_id",
    )
    .bind(guild_i64)
    .fetch_all(pool)
    .await?
    {
        let category = CategorySpec {
            guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
            category_id: i64_to_id(row.try_get::<i64, _>("category_id")?),
            name: row.try_get("name")?,
            position: row.try_get("position")?,
        };
        model.categories.insert(category.category_id, category);
    }

    for row in sqlx::query(
        "SELECT guild_id, channel_id, name, channel_type, topic, position, parent_category_id,
                nsfw, bitrate, user_limit, rate_limit_per_user, status
           FROM server_config.desired_channels
          WHERE guild_id = $1
          ORDER BY channel_id",
    )
    .bind(guild_i64)
    .fetch_all(pool)
    .await?
    {
        let channel = channel_from_row(&row)?;
        model.channels.insert(channel.channel_id, channel);
    }

    for row in sqlx::query(
        "SELECT guild_id, role_id, name, color, hoist, mentionable, managed,
                permissions_bitmask, position
           FROM server_config.desired_roles
          WHERE guild_id = $1
          ORDER BY role_id",
    )
    .bind(guild_i64)
    .fetch_all(pool)
    .await?
    {
        let role = role_from_row(&row)?;
        model.roles.insert(role.role_id, role);
    }

    for row in sqlx::query(
        "SELECT guild_id, channel_id, target_type, target_id, allow_bits, deny_bits
           FROM server_config.desired_permission_overwrites
          WHERE guild_id = $1
          ORDER BY channel_id, target_type, target_id",
    )
    .bind(guild_i64)
    .fetch_all(pool)
    .await?
    {
        let overwrite = overwrite_from_row(&row)?;
        model.overwrites.insert(overwrite.key.clone(), overwrite);
    }

    for row in sqlx::query(
        "SELECT guild_id, channel_id, message_key, message_kind, message_id, expected_hash
           FROM server_config.desired_bot_messages
          WHERE guild_id = $1
          ORDER BY channel_id, message_key",
    )
    .bind(guild_i64)
    .fetch_all(pool)
    .await?
    {
        let message = BotMessageSpec {
            guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
            channel_id: i64_to_id(row.try_get::<i64, _>("channel_id")?),
            message_key: row.try_get("message_key")?,
            message_kind: row.try_get("message_kind")?,
            message_id: optional_u64(row.try_get::<Option<i64>, _>("message_id")?),
            content_hash: row.try_get("expected_hash")?,
        };
        model
            .bot_messages
            .insert((message.channel_id, message.message_key.clone()), message);
    }

    Ok(model)
}

pub async fn load_snapshot_model(pool: &PgPool, snapshot_id: i64) -> Result<GuildModel> {
    let guild_id = sqlx::query_scalar::<_, i64>(
        "SELECT guild_id FROM server_config.live_snapshots WHERE snapshot_id = $1",
    )
    .bind(snapshot_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ServerAsCodeError::SnapshotNotFound(snapshot_id))?;

    let mut model = GuildModel::new(i64_to_id(guild_id));

    for row in sqlx::query(
        "SELECT guild_id, category_id, name, position
           FROM server_config.live_snapshot_categories
          WHERE snapshot_id = $1
          ORDER BY category_id",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?
    {
        let category = CategorySpec {
            guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
            category_id: i64_to_id(row.try_get::<i64, _>("category_id")?),
            name: row.try_get("name")?,
            position: row.try_get("position")?,
        };
        model.categories.insert(category.category_id, category);
    }

    for row in sqlx::query(
        "SELECT guild_id, channel_id, name, channel_type, topic, position, parent_category_id,
                nsfw, bitrate, user_limit, rate_limit_per_user, status
           FROM server_config.live_snapshot_channels
          WHERE snapshot_id = $1
          ORDER BY channel_id",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?
    {
        let channel = channel_from_row(&row)?;
        model.channels.insert(channel.channel_id, channel);
    }

    for row in sqlx::query(
        "SELECT guild_id, role_id, name, color, hoist, mentionable, managed,
                permissions_bitmask, position
           FROM server_config.live_snapshot_roles
          WHERE snapshot_id = $1
          ORDER BY role_id",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?
    {
        let role = role_from_row(&row)?;
        model.roles.insert(role.role_id, role);
    }

    for row in sqlx::query(
        "SELECT guild_id, channel_id, target_type, target_id, allow_bits, deny_bits
           FROM server_config.live_snapshot_permission_overwrites
          WHERE snapshot_id = $1
          ORDER BY channel_id, target_type, target_id",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?
    {
        let overwrite = overwrite_from_row(&row)?;
        model.overwrites.insert(overwrite.key.clone(), overwrite);
    }

    for row in sqlx::query(
        "SELECT guild_id, channel_id, message_key, message_kind, message_id, observed_hash
           FROM server_config.live_snapshot_bot_messages
          WHERE snapshot_id = $1
          ORDER BY channel_id, message_key",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?
    {
        let message = BotMessageSpec {
            guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
            channel_id: i64_to_id(row.try_get::<i64, _>("channel_id")?),
            message_key: row.try_get("message_key")?,
            message_kind: row.try_get("message_kind")?,
            message_id: optional_u64(row.try_get::<Option<i64>, _>("message_id")?),
            content_hash: row.try_get("observed_hash")?,
        };
        model
            .bot_messages
            .insert((message.channel_id, message.message_key.clone()), message);
    }

    Ok(model)
}

pub async fn load_dynamic_namespaces(
    pool: &PgPool,
    guild_id: DiscordId,
) -> Result<Vec<DynamicNamespace>> {
    let rows = sqlx::query(
        "SELECT namespace_id, namespace_key, system_name, match_rule_type, match_rule::text AS match_rule
           FROM server_config.dynamic_namespaces
          WHERE guild_id = $1 AND active
          ORDER BY namespace_key",
    )
    .bind(id_to_i64(guild_id)?)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let rule_type: String = row.try_get("match_rule_type")?;
            let rule_text: String = row.try_get("match_rule")?;
            let rule_json: Value = serde_json::from_str(&rule_text)?;
            Ok(DynamicNamespace {
                namespace_id: Some(row.try_get("namespace_id")?),
                namespace_key: row.try_get("namespace_key")?,
                system_name: row.try_get("system_name")?,
                match_rule: namespace_match_from_db(&rule_type, &rule_json)?,
            })
        })
        .collect()
}

pub async fn load_documented_exceptions(
    pool: &PgPool,
    guild_id: DiscordId,
) -> Result<Vec<DocumentedException>> {
    let rows = sqlx::query(
        "SELECT exception_id, exception_key, object_kind, channel_id, target_type, target_id,
                allow_bits, deny_bits, reason
           FROM server_config.documented_exceptions
          WHERE guild_id = $1 AND active
          ORDER BY exception_key",
    )
    .bind(id_to_i64(guild_id)?)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let target_type: Option<String> = row.try_get("target_type")?;
            Ok(DocumentedException {
                exception_id: Some(row.try_get("exception_id")?),
                exception_key: row.try_get("exception_key")?,
                object_kind: ObjectKind::from_db(
                    row.try_get::<String, _>("object_kind")?.as_str(),
                )?,
                channel_id: optional_u64(row.try_get::<Option<i64>, _>("channel_id")?),
                target_kind: target_type
                    .as_deref()
                    .map(TargetKind::from_db)
                    .transpose()?,
                target_id: optional_u64(row.try_get::<Option<i64>, _>("target_id")?),
                allow_bits: optional_bitmask(row.try_get::<Option<i64>, _>("allow_bits")?),
                deny_bits: optional_bitmask(row.try_get::<Option<i64>, _>("deny_bits")?),
                reason: row.try_get("reason")?,
            })
        })
        .collect()
}

pub async fn persist_diff_preview(
    pool: &PgPool,
    snapshot_id: Option<i64>,
    diff: &ServerDiff,
    created_by_user_id: Option<DiscordId>,
) -> Result<DiffPreview> {
    let diff_json = serde_json::to_string(diff)?;
    let diff_hash = diff_hash(diff)?;
    let human_summary = format::human_summary(diff);
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
    .bind(&human_summary)
    .bind(optional_id(created_by_user_id)?)
    .fetch_one(pool)
    .await?;

    Ok(DiffPreview {
        preview_id,
        guild_id: diff.guild_id,
        snapshot_id: created_snapshot_id,
        diff_hash,
        human_summary,
        diff: diff.clone(),
    })
}

pub(crate) async fn load_preview(pool: &PgPool, preview_id: i64) -> Result<DiffPreview> {
    let row = sqlx::query(
        "SELECT preview_id, guild_id, snapshot_id, diff_hash, diff_json::text AS diff_json, human_summary
           FROM server_config.diff_previews
          WHERE preview_id = $1",
    )
    .bind(preview_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ServerAsCodeError::PreviewNotFound(preview_id))?;

    let diff_json: String = row.try_get("diff_json")?;
    let diff = serde_json::from_str(&diff_json)?;
    let stored_hash: String = row.try_get("diff_hash")?;
    let recomputed_hash = diff_hash(&diff)?;
    if stored_hash != recomputed_hash {
        return Err(ServerAsCodeError::DiffHashMismatch {
            expected: stored_hash,
            actual: recomputed_hash,
        });
    }
    Ok(DiffPreview {
        preview_id: row.try_get("preview_id")?,
        guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
        snapshot_id: row.try_get("snapshot_id")?,
        diff_hash: stored_hash,
        human_summary: row.try_get("human_summary")?,
        diff,
    })
}

pub(crate) fn diff_hash(diff: &ServerDiff) -> Result<String> {
    let json = serde_json::to_vec(diff)?;
    let digest = Sha256::digest(json);
    Ok(hex::encode(digest))
}

pub(crate) async fn record_created_object_id(
    pool: &PgPool,
    change: &DiffChange,
    created_id: DiscordId,
) -> Result<()> {
    if change.action != DiffAction::Create || change.object.object_id == created_id {
        return Ok(());
    }

    let guild_id = id_to_i64(change.object.guild_id)?;
    let old_id = id_to_i64(change.object.object_id)?;
    let new_id = id_to_i64(created_id)?;
    let mut tx = pool.begin().await?;

    match change.object.kind {
        ObjectKind::Category => {
            sqlx::query(
                "UPDATE server_config.desired_categories
                    SET category_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND category_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE server_config.desired_channels
                    SET parent_category_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND parent_category_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
        }
        ObjectKind::Channel => {
            sqlx::query(
                "UPDATE server_config.desired_channels
                    SET channel_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND channel_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE server_config.desired_permission_overwrites
                    SET channel_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND channel_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE server_config.desired_bot_messages
                    SET channel_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND channel_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
        }
        ObjectKind::Role => {
            sqlx::query(
                "UPDATE server_config.desired_roles
                    SET role_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND role_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE server_config.desired_permission_overwrites
                    SET target_id = $3,
                        updated_at = now()
                  WHERE guild_id = $1 AND target_type = 'role' AND target_id = $2",
            )
            .bind(guild_id)
            .bind(old_id)
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
        }
        ObjectKind::PermissionOverwrite | ObjectKind::BotMessage => {}
    }

    tx.commit().await?;
    Ok(())
}

pub async fn adopt_change(
    pool: &PgPool,
    snapshot_id: i64,
    change: &DiffChange,
    adopted_by_user_id: Option<DiscordId>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    match change.action {
        DiffAction::Create => delete_desired_object(&mut tx, change).await?,
        DiffAction::Update | DiffAction::Delete => {
            upsert_actual_as_desired(&mut tx, change).await?
        }
    }

    sqlx::query(
        "INSERT INTO server_config.adoption_events
         (guild_id, snapshot_id, adopted_by_user_id, object_kind, object_id, adopted_diff)
         VALUES ($1, $2, $3, $4, $5, $6::text::jsonb)",
    )
    .bind(id_to_i64(change.object.guild_id)?)
    .bind(snapshot_id)
    .bind(optional_id(adopted_by_user_id)?)
    .bind(change.object.kind.as_db())
    .bind(id_to_i64(change.object.object_id)?)
    .bind(serde_json::to_string(&diff_json(change))?)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

async fn upsert_actual_as_desired(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    change: &DiffChange,
) -> Result<()> {
    let Some(actual) = &change.actual else {
        return Ok(());
    };
    match change.object.kind {
        ObjectKind::Category => {
            let spec: CategorySpec = serde_json::from_value(actual.clone())?;
            sqlx::query(
                "INSERT INTO server_config.desired_categories
                 (guild_id, category_id, name, position)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (guild_id, category_id)
                 DO UPDATE SET name = EXCLUDED.name,
                               position = EXCLUDED.position,
                               updated_at = now()",
            )
            .bind(id_to_i64(spec.guild_id)?)
            .bind(id_to_i64(spec.category_id)?)
            .bind(spec.name)
            .bind(spec.position)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::Channel => {
            let spec: ChannelSpec = serde_json::from_value(actual.clone())?;
            upsert_desired_channel(tx, &spec).await?;
        }
        ObjectKind::Role => {
            let spec: RoleSpec = serde_json::from_value(actual.clone())?;
            sqlx::query(
                "INSERT INTO server_config.desired_roles
                 (guild_id, role_id, name, color, hoist, mentionable, managed, permissions_bitmask, position)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (guild_id, role_id)
                 DO UPDATE SET name = EXCLUDED.name,
                               color = EXCLUDED.color,
                               hoist = EXCLUDED.hoist,
                               mentionable = EXCLUDED.mentionable,
                               managed = EXCLUDED.managed,
                               permissions_bitmask = EXCLUDED.permissions_bitmask,
                               position = EXCLUDED.position,
                               updated_at = now()",
            )
            .bind(id_to_i64(spec.guild_id)?)
            .bind(id_to_i64(spec.role_id)?)
            .bind(spec.name)
            .bind(spec.color)
            .bind(spec.hoist)
            .bind(spec.mentionable)
            .bind(spec.managed)
            .bind(bitmask_to_i64(spec.permissions_bitmask)?)
            .bind(spec.position)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::PermissionOverwrite => {
            let spec: PermissionOverwriteSpec = serde_json::from_value(actual.clone())?;
            sqlx::query(
                "INSERT INTO server_config.desired_permission_overwrites
                 (guild_id, channel_id, target_type, target_id, allow_bits, deny_bits)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (guild_id, channel_id, target_type, target_id)
                 DO UPDATE SET allow_bits = EXCLUDED.allow_bits,
                               deny_bits = EXCLUDED.deny_bits,
                               updated_at = now()",
            )
            .bind(id_to_i64(spec.guild_id)?)
            .bind(id_to_i64(spec.key.channel_id)?)
            .bind(spec.key.target_kind.as_db())
            .bind(id_to_i64(spec.key.target_id)?)
            .bind(bitmask_to_i64(spec.allow_bits)?)
            .bind(bitmask_to_i64(spec.deny_bits)?)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::BotMessage => {
            let spec: BotMessageSpec = serde_json::from_value(actual.clone())?;
            sqlx::query(
                "INSERT INTO server_config.desired_bot_messages
                 (guild_id, channel_id, message_key, message_kind, message_id, expected_hash)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (guild_id, channel_id, message_key)
                 DO UPDATE SET message_kind = EXCLUDED.message_kind,
                               message_id = EXCLUDED.message_id,
                               expected_hash = EXCLUDED.expected_hash,
                               updated_at = now()",
            )
            .bind(id_to_i64(spec.guild_id)?)
            .bind(id_to_i64(spec.channel_id)?)
            .bind(spec.message_key)
            .bind(spec.message_kind)
            .bind(optional_id(spec.message_id)?)
            .bind(spec.content_hash)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn delete_desired_object(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    change: &DiffChange,
) -> Result<()> {
    match change.object.kind {
        ObjectKind::Category => {
            sqlx::query(
                "DELETE FROM server_config.desired_permission_overwrites overwrites
                  USING server_config.desired_channels channels
                 WHERE overwrites.guild_id = channels.guild_id
                   AND overwrites.channel_id = channels.channel_id
                   AND channels.guild_id = $1
                   AND channels.parent_category_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "DELETE FROM server_config.desired_bot_messages messages
                  USING server_config.desired_channels channels
                 WHERE messages.guild_id = channels.guild_id
                   AND messages.channel_id = channels.channel_id
                   AND channels.guild_id = $1
                   AND channels.parent_category_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "DELETE FROM server_config.desired_channels
                  WHERE guild_id = $1 AND parent_category_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "DELETE FROM server_config.desired_categories WHERE guild_id = $1 AND category_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::Channel => {
            sqlx::query(
                "DELETE FROM server_config.desired_permission_overwrites
                  WHERE guild_id = $1 AND channel_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "DELETE FROM server_config.desired_bot_messages
                  WHERE guild_id = $1 AND channel_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "DELETE FROM server_config.desired_channels WHERE guild_id = $1 AND channel_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::Role => {
            sqlx::query(
                "DELETE FROM server_config.desired_permission_overwrites
                  WHERE guild_id = $1 AND target_type = 'role' AND target_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "DELETE FROM server_config.desired_roles WHERE guild_id = $1 AND role_id = $2",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::PermissionOverwrite => {
            let Some(target_kind) = change.object.target_kind else {
                return Ok(());
            };
            let Some(target_id) = change.object.target_id else {
                return Ok(());
            };
            sqlx::query(
                "DELETE FROM server_config.desired_permission_overwrites
                  WHERE guild_id = $1 AND channel_id = $2 AND target_type = $3 AND target_id = $4",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .bind(target_kind.as_db())
            .bind(id_to_i64(target_id)?)
            .execute(&mut **tx)
            .await?;
        }
        ObjectKind::BotMessage => {
            let Some(message_key) = &change.object.message_key else {
                return Ok(());
            };
            sqlx::query(
                "DELETE FROM server_config.desired_bot_messages
                  WHERE guild_id = $1 AND channel_id = $2 AND message_key = $3",
            )
            .bind(id_to_i64(change.object.guild_id)?)
            .bind(id_to_i64(change.object.object_id)?)
            .bind(message_key)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn upsert_desired_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    spec: &ChannelSpec,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO server_config.desired_channels
         (guild_id, channel_id, name, channel_type, topic, position, parent_category_id,
          nsfw, bitrate, user_limit, rate_limit_per_user, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (guild_id, channel_id)
         DO UPDATE SET name = EXCLUDED.name,
                       channel_type = EXCLUDED.channel_type,
                       topic = EXCLUDED.topic,
                       position = EXCLUDED.position,
                       parent_category_id = EXCLUDED.parent_category_id,
                       nsfw = EXCLUDED.nsfw,
                       bitrate = EXCLUDED.bitrate,
                       user_limit = EXCLUDED.user_limit,
                       rate_limit_per_user = EXCLUDED.rate_limit_per_user,
                       status = EXCLUDED.status,
                       updated_at = now()",
    )
    .bind(id_to_i64(spec.guild_id)?)
    .bind(id_to_i64(spec.channel_id)?)
    .bind(&spec.name)
    .bind(spec.kind.as_db())
    .bind(&spec.topic)
    .bind(spec.position)
    .bind(optional_id(spec.parent_category_id)?)
    .bind(spec.nsfw)
    .bind(spec.bitrate)
    .bind(spec.user_limit)
    .bind(spec.rate_limit_per_user)
    .bind(&spec.status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn channel_from_row(row: &sqlx::postgres::PgRow) -> Result<ChannelSpec> {
    Ok(ChannelSpec {
        guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
        channel_id: i64_to_id(row.try_get::<i64, _>("channel_id")?),
        name: row.try_get("name")?,
        kind: ChannelKind::from_db(row.try_get::<String, _>("channel_type")?.as_str()),
        topic: row.try_get("topic")?,
        position: row.try_get("position")?,
        parent_category_id: optional_u64(row.try_get::<Option<i64>, _>("parent_category_id")?),
        nsfw: row.try_get("nsfw")?,
        bitrate: row.try_get("bitrate")?,
        user_limit: row.try_get("user_limit")?,
        rate_limit_per_user: row.try_get("rate_limit_per_user")?,
        status: row.try_get("status")?,
    })
}

fn role_from_row(row: &sqlx::postgres::PgRow) -> Result<RoleSpec> {
    Ok(RoleSpec {
        guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
        role_id: i64_to_id(row.try_get::<i64, _>("role_id")?),
        name: row.try_get("name")?,
        color: row.try_get("color")?,
        hoist: row.try_get("hoist")?,
        mentionable: row.try_get("mentionable")?,
        managed: row.try_get("managed")?,
        permissions_bitmask: i64_to_bitmask(row.try_get("permissions_bitmask")?),
        position: row.try_get("position")?,
    })
}

fn overwrite_from_row(row: &sqlx::postgres::PgRow) -> Result<PermissionOverwriteSpec> {
    let target_kind = TargetKind::from_db(row.try_get::<String, _>("target_type")?.as_str())?;
    Ok(PermissionOverwriteSpec {
        guild_id: i64_to_id(row.try_get::<i64, _>("guild_id")?),
        key: OverwriteKey {
            channel_id: i64_to_id(row.try_get::<i64, _>("channel_id")?),
            target_kind,
            target_id: i64_to_id(row.try_get::<i64, _>("target_id")?),
        },
        allow_bits: i64_to_bitmask(row.try_get("allow_bits")?),
        deny_bits: i64_to_bitmask(row.try_get("deny_bits")?),
    })
}

fn namespace_match_from_db(rule_type: &str, rule_json: &Value) -> Result<NamespaceMatch> {
    match rule_type {
        "parent_category" => Ok(NamespaceMatch::ParentCategory(json_id(
            rule_json,
            "parent_category_id",
        )?)),
        "name_prefix" => Ok(NamespaceMatch::NamePrefix(json_string(
            rule_json, "prefix",
        )?)),
        "name_pattern" => Ok(NamespaceMatch::NamePattern(json_string(
            rule_json, "pattern",
        )?)),
        "channel_id" => Ok(NamespaceMatch::ChannelId(json_id(rule_json, "channel_id")?)),
        other => Err(ServerAsCodeError::UnknownChannelKind(other.to_string())),
    }
}

fn json_id(value: &Value, key: &str) -> Result<u64> {
    if let Some(id) = value.get(key).and_then(Value::as_u64) {
        return Ok(id);
    }
    if let Some(id) = value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Ok(id);
    }
    Ok(0)
}

fn json_string(value: &Value, key: &str) -> Result<String> {
    Ok(value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn optional_id(value: Option<u64>) -> Result<Option<i64>> {
    value.map(id_to_i64).transpose()
}

fn optional_u64(value: Option<i64>) -> Option<u64> {
    value.map(i64_to_id)
}

fn optional_bitmask(value: Option<i64>) -> Option<u64> {
    value.map(i64_to_bitmask)
}
