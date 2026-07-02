use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::Permissions;

use crate::model::{
    BotMessageSpec, CategorySpec, ChannelSpec, DiscordId, DocumentedException, DynamicNamespace,
    GuildModel, ObjectKind, ObjectRef, OverwriteKey, PermissionOverwriteSpec, RoleSpec, TargetKind,
};
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffAction {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDiff {
    pub field: String,
    pub desired: Value,
    pub actual: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffChange {
    pub object: ObjectRef,
    pub action: DiffAction,
    pub fields: Vec<FieldDiff>,
    pub desired: Option<Value>,
    pub actual: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FilterReason {
    DynamicNamespace {
        namespace_key: String,
        system_name: String,
    },
    DocumentedException {
        exception_key: String,
        exception_id: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilteredDiff {
    pub change: DiffChange,
    pub reason: FilterReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedDiff {
    pub change: DiffChange,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDiff {
    pub guild_id: DiscordId,
    pub changes: Vec<DiffChange>,
    pub filtered: Vec<FilteredDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<BlockedDiff>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub compare_relative_positions: bool,
}

impl ServerDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.blocked.is_empty()
    }
}

pub fn diff_models(
    desired: &GuildModel,
    actual: &GuildModel,
    dynamic_namespaces: &[DynamicNamespace],
    documented_exceptions: &[DocumentedException],
) -> Result<ServerDiff> {
    diff_models_with_options(
        desired,
        actual,
        dynamic_namespaces,
        documented_exceptions,
        DiffOptions::default(),
    )
}

pub fn diff_models_with_options(
    desired: &GuildModel,
    actual: &GuildModel,
    dynamic_namespaces: &[DynamicNamespace],
    documented_exceptions: &[DocumentedException],
    options: DiffOptions,
) -> Result<ServerDiff> {
    let mut raw = Vec::new();
    diff_categories(desired, actual, &mut raw)?;
    diff_channels(desired, actual, &mut raw)?;
    diff_roles(desired, actual, &mut raw)?;
    diff_overwrites(desired, actual, &mut raw)?;
    diff_bot_messages(desired, actual, &mut raw)?;
    if options.compare_relative_positions {
        diff_relative_category_order(desired, actual, &mut raw)?;
        diff_relative_channel_order(desired, actual, &mut raw)?;
        diff_relative_role_order(desired, actual, &mut raw)?;
    }

    let mut changes = Vec::new();
    let mut filtered = Vec::new();
    let mut blocked = Vec::new();
    for change in raw {
        if let Some(reason) = filter_reason(
            &change,
            desired,
            actual,
            dynamic_namespaces,
            documented_exceptions,
        )? {
            filtered.push(FilteredDiff { change, reason });
        } else if let Some(reason) = effective_rights_block_reason(&change, desired, actual)? {
            blocked.push(BlockedDiff { change, reason });
        } else {
            changes.push(change);
        }
    }

    Ok(ServerDiff {
        guild_id: desired.guild_id,
        changes,
        filtered,
        blocked,
    })
}

fn diff_categories(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    for (id, desired_spec) in &desired.categories {
        match actual.categories.get(id) {
            Some(actual_spec) => {
                let fields = category_fields(desired_spec, actual_spec);
                if !fields.is_empty() {
                    changes.push(update_change(
                        ObjectRef::category(desired_spec),
                        fields,
                        desired_spec,
                        actual_spec,
                    )?);
                }
            }
            None => changes.push(create_change(
                ObjectRef::category(desired_spec),
                desired_spec,
            )?),
        }
    }
    for (id, actual_spec) in &actual.categories {
        if !desired.categories.contains_key(id) {
            changes.push(delete_change(
                ObjectRef::category(actual_spec),
                actual_spec,
            )?);
        }
    }
    Ok(())
}

fn diff_channels(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    for (id, desired_spec) in &desired.channels {
        match actual.channels.get(id) {
            Some(actual_spec) => {
                let fields = channel_fields(desired_spec, actual_spec);
                if !fields.is_empty() {
                    changes.push(update_change(
                        ObjectRef::channel(desired_spec),
                        fields,
                        desired_spec,
                        actual_spec,
                    )?);
                }
            }
            None => changes.push(create_change(
                ObjectRef::channel(desired_spec),
                desired_spec,
            )?),
        }
    }
    for (id, actual_spec) in &actual.channels {
        if !desired.channels.contains_key(id) {
            changes.push(delete_change(ObjectRef::channel(actual_spec), actual_spec)?);
        }
    }
    Ok(())
}

fn diff_roles(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    for (id, desired_spec) in &desired.roles {
        match actual.roles.get(id) {
            Some(actual_spec) => {
                let fields = role_fields(desired_spec, actual_spec);
                if !fields.is_empty() {
                    changes.push(update_change(
                        ObjectRef::role(desired_spec),
                        fields,
                        desired_spec,
                        actual_spec,
                    )?);
                }
            }
            None => changes.push(create_change(ObjectRef::role(desired_spec), desired_spec)?),
        }
    }
    for (id, actual_spec) in &actual.roles {
        if !desired.roles.contains_key(id) {
            changes.push(delete_change(ObjectRef::role(actual_spec), actual_spec)?);
        }
    }
    Ok(())
}

fn diff_overwrites(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    for (key, desired_spec) in &desired.overwrites {
        match actual.overwrites.get(key) {
            Some(actual_spec) => {
                let fields = overwrite_fields(desired_spec, actual_spec);
                if !fields.is_empty() {
                    changes.push(update_change(
                        ObjectRef::overwrite(desired_spec),
                        fields,
                        desired_spec,
                        actual_spec,
                    )?);
                }
            }
            None => changes.push(create_change(
                ObjectRef::overwrite(desired_spec),
                desired_spec,
            )?),
        }
    }
    for (key, actual_spec) in &actual.overwrites {
        if !desired.overwrites.contains_key(key) {
            changes.push(delete_change(
                ObjectRef::overwrite(actual_spec),
                actual_spec,
            )?);
        }
    }
    Ok(())
}

fn diff_bot_messages(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    for (key, desired_spec) in &desired.bot_messages {
        match actual.bot_messages.get(key) {
            Some(actual_spec) => {
                let fields = bot_message_fields(desired_spec, actual_spec);
                if !fields.is_empty() {
                    changes.push(update_change(
                        ObjectRef::bot_message(desired_spec),
                        fields,
                        desired_spec,
                        actual_spec,
                    )?);
                }
            }
            None => {
                changes.push(create_change(
                    ObjectRef::bot_message(desired_spec),
                    desired_spec,
                )?);
            }
        }
    }
    for (key, actual_spec) in &actual.bot_messages {
        if !desired.bot_messages.contains_key(key) {
            changes.push(delete_change(
                ObjectRef::bot_message(actual_spec),
                actual_spec,
            )?);
        }
    }
    Ok(())
}

fn category_fields(desired: &CategorySpec, actual: &CategorySpec) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    push_diff(&mut fields, "name", &desired.name, &actual.name);
    fields
}

fn channel_fields(desired: &ChannelSpec, actual: &ChannelSpec) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    push_diff(&mut fields, "name", &desired.name, &actual.name);
    push_diff(&mut fields, "kind", &desired.kind, &actual.kind);
    push_diff(&mut fields, "topic", &desired.topic, &actual.topic);
    push_diff(
        &mut fields,
        "parent_category_id",
        desired.parent_category_id,
        actual.parent_category_id,
    );
    push_diff(&mut fields, "nsfw", desired.nsfw, actual.nsfw);
    push_diff(&mut fields, "bitrate", desired.bitrate, actual.bitrate);
    push_diff(
        &mut fields,
        "user_limit",
        desired.user_limit,
        actual.user_limit,
    );
    push_diff(
        &mut fields,
        "rate_limit_per_user",
        desired.rate_limit_per_user,
        actual.rate_limit_per_user,
    );
    push_diff(&mut fields, "status", &desired.status, &actual.status);
    fields
}

fn role_fields(desired: &RoleSpec, actual: &RoleSpec) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    push_diff(&mut fields, "name", &desired.name, &actual.name);
    push_diff(&mut fields, "color", desired.color, actual.color);
    push_diff(&mut fields, "hoist", desired.hoist, actual.hoist);
    push_diff(
        &mut fields,
        "mentionable",
        desired.mentionable,
        actual.mentionable,
    );
    push_diff(&mut fields, "managed", desired.managed, actual.managed);
    push_diff(
        &mut fields,
        "permissions_bitmask",
        desired.permissions_bitmask,
        actual.permissions_bitmask,
    );
    fields
}

fn diff_relative_category_order(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    let desired_ids = category_order(desired);
    let actual_ids = category_order(actual);
    for id in common_ids_with_changed_index(&desired_ids, &actual_ids) {
        if let (Some(desired_spec), Some(actual_spec)) =
            (desired.categories.get(&id), actual.categories.get(&id))
        {
            push_relative_order_change(
                changes,
                ObjectRef::category(desired_spec),
                desired_spec,
                actual_spec,
                desired_ids.iter().position(|candidate| *candidate == id),
                actual_ids.iter().position(|candidate| *candidate == id),
            )?;
        }
    }
    Ok(())
}

fn diff_relative_channel_order(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    let mut parent_ids: Vec<_> = desired
        .channels
        .values()
        .filter_map(|channel| channel.parent_category_id)
        .chain(
            actual
                .channels
                .values()
                .filter_map(|channel| channel.parent_category_id),
        )
        .collect();
    parent_ids.push(0);
    parent_ids.sort_unstable();
    parent_ids.dedup();

    for parent_id in parent_ids {
        let parent = (parent_id != 0).then_some(parent_id);
        let desired_ids = channel_order(desired, parent);
        let actual_ids = channel_order(actual, parent);
        for id in common_ids_with_changed_index(&desired_ids, &actual_ids) {
            if let (Some(desired_spec), Some(actual_spec)) =
                (desired.channels.get(&id), actual.channels.get(&id))
            {
                push_relative_order_change(
                    changes,
                    ObjectRef::channel(desired_spec),
                    desired_spec,
                    actual_spec,
                    desired_ids.iter().position(|candidate| *candidate == id),
                    actual_ids.iter().position(|candidate| *candidate == id),
                )?;
            }
        }
    }
    Ok(())
}

fn diff_relative_role_order(
    desired: &GuildModel,
    actual: &GuildModel,
    changes: &mut Vec<DiffChange>,
) -> Result<()> {
    let desired_ids = role_order(desired);
    let actual_ids = role_order(actual);
    for id in common_ids_with_changed_index(&desired_ids, &actual_ids) {
        if let (Some(desired_spec), Some(actual_spec)) =
            (desired.roles.get(&id), actual.roles.get(&id))
        {
            push_relative_order_change(
                changes,
                ObjectRef::role(desired_spec),
                desired_spec,
                actual_spec,
                desired_ids.iter().position(|candidate| *candidate == id),
                actual_ids.iter().position(|candidate| *candidate == id),
            )?;
        }
    }
    Ok(())
}

fn category_order(model: &GuildModel) -> Vec<DiscordId> {
    let mut categories: Vec<_> = model.categories.values().collect();
    categories.sort_by_key(|category| (category.position, category.category_id));
    categories
        .into_iter()
        .map(|category| category.category_id)
        .collect()
}

fn channel_order(model: &GuildModel, parent: Option<DiscordId>) -> Vec<DiscordId> {
    let mut channels: Vec<_> = model
        .channels
        .values()
        .filter(|channel| channel.parent_category_id == parent)
        .collect();
    channels.sort_by_key(|channel| (channel.position, channel.channel_id));
    channels
        .into_iter()
        .map(|channel| channel.channel_id)
        .collect()
}

fn role_order(model: &GuildModel) -> Vec<DiscordId> {
    let mut roles: Vec<_> = model.roles.values().collect();
    roles.sort_by_key(|role| (role.position, role.role_id));
    roles.into_iter().map(|role| role.role_id).collect()
}

fn common_ids_with_changed_index(
    desired_ids: &[DiscordId],
    actual_ids: &[DiscordId],
) -> Vec<DiscordId> {
    desired_ids
        .iter()
        .enumerate()
        .filter_map(|(desired_index, id)| {
            let actual_index = actual_ids.iter().position(|candidate| candidate == id)?;
            (actual_index != desired_index).then_some(*id)
        })
        .collect()
}

fn push_relative_order_change<T: Serialize>(
    changes: &mut Vec<DiffChange>,
    object: ObjectRef,
    desired: &T,
    actual: &T,
    desired_index: Option<usize>,
    actual_index: Option<usize>,
) -> Result<()> {
    let field = FieldDiff {
        field: "relative_position".to_string(),
        desired: serde_json::to_value(desired_index)?,
        actual: serde_json::to_value(actual_index)?,
    };

    if let Some(change) = changes.iter_mut().find(|change| {
        change.object == object
            && change.action == DiffAction::Update
            && change.desired.is_some()
            && change.actual.is_some()
    }) {
        change.fields.push(field);
        return Ok(());
    }

    changes.push(update_change(object, vec![field], desired, actual)?);
    Ok(())
}

fn overwrite_fields(
    desired: &PermissionOverwriteSpec,
    actual: &PermissionOverwriteSpec,
) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    push_diff(
        &mut fields,
        "allow_bits",
        desired.allow_bits,
        actual.allow_bits,
    );
    push_diff(
        &mut fields,
        "deny_bits",
        desired.deny_bits,
        actual.deny_bits,
    );
    fields
}

fn bot_message_fields(desired: &BotMessageSpec, actual: &BotMessageSpec) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    push_diff(
        &mut fields,
        "message_kind",
        &desired.message_kind,
        &actual.message_kind,
    );
    push_diff(
        &mut fields,
        "message_id",
        desired.message_id,
        actual.message_id,
    );
    push_diff(
        &mut fields,
        "content_hash",
        &desired.content_hash,
        &actual.content_hash,
    );
    fields
}

fn push_diff<T>(fields: &mut Vec<FieldDiff>, field: &str, desired: T, actual: T)
where
    T: Serialize + PartialEq,
{
    if desired == actual {
        return;
    }
    fields.push(FieldDiff {
        field: field.to_string(),
        desired: serde_json::to_value(desired).unwrap_or(Value::Null),
        actual: serde_json::to_value(actual).unwrap_or(Value::Null),
    });
}

fn create_change<T: Serialize>(object: ObjectRef, desired: &T) -> Result<DiffChange> {
    Ok(DiffChange {
        object,
        action: DiffAction::Create,
        fields: Vec::new(),
        desired: Some(serde_json::to_value(desired)?),
        actual: None,
    })
}

fn update_change<T: Serialize>(
    object: ObjectRef,
    fields: Vec<FieldDiff>,
    desired: &T,
    actual: &T,
) -> Result<DiffChange> {
    Ok(DiffChange {
        object,
        action: DiffAction::Update,
        fields,
        desired: Some(serde_json::to_value(desired)?),
        actual: Some(serde_json::to_value(actual)?),
    })
}

fn delete_change<T: Serialize>(object: ObjectRef, actual: &T) -> Result<DiffChange> {
    Ok(DiffChange {
        object,
        action: DiffAction::Delete,
        fields: Vec::new(),
        desired: None,
        actual: Some(serde_json::to_value(actual)?),
    })
}

fn filter_reason(
    change: &DiffChange,
    desired: &GuildModel,
    actual: &GuildModel,
    dynamic_namespaces: &[DynamicNamespace],
    documented_exceptions: &[DocumentedException],
) -> Result<Option<FilterReason>> {
    if let Some(reason) = documented_exception_filter(change, documented_exceptions)? {
        return Ok(Some(reason));
    }
    dynamic_namespace_filter(change, desired, actual, dynamic_namespaces)
}

fn effective_rights_block_reason(
    change: &DiffChange,
    desired: &GuildModel,
    actual: &GuildModel,
) -> Result<Option<String>> {
    if change.object.kind != ObjectKind::PermissionOverwrite || change.action != DiffAction::Delete
    {
        return Ok(None);
    }
    let Some(actual_value) = change.actual.as_ref() else {
        return Ok(None);
    };
    let actual_spec: PermissionOverwriteSpec = serde_json::from_value(actual_value.clone())?;
    let channel_id = actual_spec.key.channel_id;
    if !actual.channels.contains_key(&channel_id) && !desired.channels.contains_key(&channel_id) {
        return Ok(None);
    }
    if desired_channel_materializes_visibility_deny(desired, channel_id) {
        return Ok(None);
    }

    let before = effective_channel_permissions(
        actual,
        channel_id,
        actual_spec.key.target_kind,
        actual_spec.key.target_id,
    );
    let after = effective_channel_permissions(
        desired,
        channel_id,
        actual_spec.key.target_kind,
        actual_spec.key.target_id,
    );
    if before == after {
        return Ok(None);
    }

    Ok(Some(format!(
        "effektive Rechte würden sich ändern: {}: {before:#x} -> {after:#x}",
        subject_label(
            actual,
            actual_spec.key.target_kind,
            actual_spec.key.target_id
        )
    )))
}

fn desired_channel_materializes_visibility_deny(model: &GuildModel, channel_id: DiscordId) -> bool {
    model
        .overwrites
        .get(&OverwriteKey {
            channel_id,
            target_kind: TargetKind::Role,
            target_id: model.guild_id,
        })
        .is_some_and(|overwrite| {
            Permissions::from_bits_truncate(overwrite.deny_bits).contains(Permissions::VIEW_CHANNEL)
        })
}

fn effective_channel_permissions(
    model: &GuildModel,
    channel_id: DiscordId,
    target_kind: TargetKind,
    target_id: DiscordId,
) -> u64 {
    let mut permissions = model
        .roles
        .get(&model.guild_id)
        .or_else(|| model.roles.values().find(|role| role.name == "@everyone"))
        .map_or(0, |role| role.permissions_bitmask);

    if target_kind == TargetKind::Role && target_id != model.guild_id {
        permissions |= model
            .roles
            .get(&target_id)
            .map_or(0, |role| role.permissions_bitmask);
    }

    if Permissions::from_bits_truncate(permissions).contains(Permissions::ADMINISTRATOR) {
        return Permissions::all().bits();
    }

    if let Some(overwrite) = model.overwrites.get(&OverwriteKey {
        channel_id,
        target_kind: TargetKind::Role,
        target_id: model.guild_id,
    }) {
        permissions = apply_overwrite_bits(permissions, overwrite);
    }

    if target_kind == TargetKind::Role && target_id != model.guild_id {
        if let Some(overwrite) = model.overwrites.get(&OverwriteKey {
            channel_id,
            target_kind: TargetKind::Role,
            target_id,
        }) {
            permissions = apply_overwrite_bits(permissions, overwrite);
        }
    }

    if target_kind == TargetKind::Member {
        if let Some(overwrite) = model.overwrites.get(&OverwriteKey {
            channel_id,
            target_kind: TargetKind::Member,
            target_id,
        }) {
            permissions = apply_overwrite_bits(permissions, overwrite);
        }
    }

    permissions
}

fn apply_overwrite_bits(base: u64, overwrite: &PermissionOverwriteSpec) -> u64 {
    (base & !overwrite.deny_bits) | overwrite.allow_bits
}

fn subject_label(model: &GuildModel, target_kind: TargetKind, target_id: DiscordId) -> String {
    match target_kind {
        TargetKind::Role if target_id == model.guild_id => "@everyone".to_string(),
        TargetKind::Role => format!("role:{target_id}"),
        TargetKind::Member => format!("member:{target_id}"),
    }
}

fn documented_exception_filter(
    change: &DiffChange,
    documented_exceptions: &[DocumentedException],
) -> Result<Option<FilterReason>> {
    if change.object.kind != ObjectKind::PermissionOverwrite {
        return Ok(None);
    }

    let Some(actual) = change.actual.as_ref() else {
        return Ok(None);
    };
    let spec: PermissionOverwriteSpec = serde_json::from_value(actual.clone())?;
    if let Some(exception) = documented_exceptions
        .iter()
        .find(|exception| exception.matches_overwrite(&spec))
    {
        return Ok(Some(FilterReason::DocumentedException {
            exception_key: exception.exception_key.clone(),
            exception_id: exception.exception_id,
        }));
    }
    Ok(None)
}

fn dynamic_namespace_filter(
    change: &DiffChange,
    desired: &GuildModel,
    actual: &GuildModel,
    dynamic_namespaces: &[DynamicNamespace],
) -> Result<Option<FilterReason>> {
    let Some(channel_id) = change.object.channel_id else {
        return Ok(None);
    };

    for namespace in dynamic_namespaces {
        if namespace.matches_channel(actual, channel_id)?
            || namespace.matches_channel(desired, channel_id)?
        {
            match change.object.kind {
                ObjectKind::Channel
                    if matches!(change.action, DiffAction::Create | DiffAction::Delete) => {}
                ObjectKind::PermissionOverwrite if namespace_allows_free_overwrites(namespace) => {}
                _ => continue,
            }
            return Ok(Some(FilterReason::DynamicNamespace {
                namespace_key: namespace.namespace_key.clone(),
                system_name: namespace.system_name.clone(),
            }));
        }
    }
    Ok(None)
}

fn namespace_allows_free_overwrites(namespace: &DynamicNamespace) -> bool {
    // §3.11: TempVoice-Rechte werden je Lane-Einstellung von dl-voice verwaltet.
    // faq-* und Coaching/Scrim-Teamkanaele sind ebenfalls runtime-/projektverwaltet.
    // Tickets und Bot-Pate-Fallbacks haben dagegen dokumentierte Rechte-Muster und werden geprueft.
    namespace.system_name == "dl-voice"
        || namespace.namespace_key.starts_with("tempvoice_")
        || namespace.namespace_key == "faq_channels"
        || namespace.namespace_key == "coaching_scrim_team_channels"
}

pub(crate) fn diff_json(change: &DiffChange) -> Value {
    json!({
        "object": change.object,
        "action": change.action,
        "fields": change.fields,
        "desired": change.desired,
        "actual": change.actual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChannelKind, OverwriteKey, RoleSpec};

    const GUILD_ID: u64 = 1_289_721_245_281_292_288;
    const CHANNEL_ID: u64 = 10;
    const ROLE_ID: u64 = 20;

    fn role(id: u64, name: &str, permissions: Permissions) -> RoleSpec {
        RoleSpec {
            guild_id: GUILD_ID,
            role_id: id,
            name: name.to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: permissions.bits(),
            position: 0,
        }
    }

    fn model_with_channel() -> GuildModel {
        let mut model = GuildModel::new(GUILD_ID);
        model.roles.insert(
            GUILD_ID,
            role(
                GUILD_ID,
                "@everyone",
                Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            ),
        );
        model
            .roles
            .insert(ROLE_ID, role(ROLE_ID, "Privat", Permissions::empty()));
        model.channels.insert(
            CHANNEL_ID,
            ChannelSpec {
                guild_id: GUILD_ID,
                channel_id: CHANNEL_ID,
                name: "privat".to_string(),
                kind: ChannelKind::Text,
                topic: None,
                position: 0,
                parent_category_id: None,
                nsfw: false,
                bitrate: None,
                user_limit: None,
                rate_limit_per_user: None,
                status: None,
            },
        );
        model
    }

    fn overwrite(
        target_kind: TargetKind,
        target_id: u64,
        allow: Permissions,
        deny: Permissions,
    ) -> PermissionOverwriteSpec {
        PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: CHANNEL_ID,
                target_kind,
                target_id,
            },
            allow_bits: allow.bits(),
            deny_bits: deny.bits(),
        }
    }

    #[test]
    fn overwrite_delete_ohne_materialisierten_ersatz_wird_wegen_effektiver_rechte_geblockt(
    ) -> anyhow::Result<()> {
        let desired = model_with_channel();
        let mut actual = model_with_channel();
        let hidden = overwrite(
            TargetKind::Role,
            GUILD_ID,
            Permissions::empty(),
            Permissions::VIEW_CHANNEL,
        );
        actual.overwrites.insert(hidden.key.clone(), hidden);

        let diff = diff_models(&desired, &actual, &[], &[])?;

        assert!(diff.changes.is_empty(), "{:?}", diff.changes);
        assert_eq!(diff.blocked.len(), 1);
        assert!(diff.blocked[0]
            .reason
            .contains("effektive Rechte würden sich ändern: @everyone"));
        Ok(())
    }

    #[test]
    fn overwrite_delete_mit_materialisiertem_view_deny_bleibt_ausfuehrbar() -> anyhow::Result<()> {
        let mut desired = model_with_channel();
        let materialized = overwrite(
            TargetKind::Role,
            GUILD_ID,
            Permissions::empty(),
            Permissions::VIEW_CHANNEL,
        );
        desired
            .overwrites
            .insert(materialized.key.clone(), materialized.clone());

        let mut actual = model_with_channel();
        actual
            .overwrites
            .insert(materialized.key.clone(), materialized);
        let legacy_role_allow = overwrite(
            TargetKind::Role,
            ROLE_ID,
            Permissions::VIEW_CHANNEL,
            Permissions::empty(),
        );
        actual
            .overwrites
            .insert(legacy_role_allow.key.clone(), legacy_role_allow);

        let diff = diff_models(&desired, &actual, &[], &[])?;

        assert!(diff.blocked.is_empty(), "{:?}", diff.blocked);
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(diff.changes[0].action, DiffAction::Delete);
        assert_eq!(diff.changes[0].object.target_id, Some(ROLE_ID));
        Ok(())
    }
}
