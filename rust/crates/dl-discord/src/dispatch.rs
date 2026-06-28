//! Interaction-Dispatch: Gateway-Interactions → [`InteractionRouter`] → Response.
//!
//! Antwortet über die Raw-HTTP-Endpunkte (JSON statt Builder), damit die
//! deklarativen [`BridgeReply`]-Strukturen 1:1 durchgereicht werden können.
//! Verhalten wie das Python-Original: Antworten, die länger als 2 Sekunden
//! brauchen, werden per Defer + Followup geliefert.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use serenity::all::{
    CommandDataOption, CommandDataOptionValue, CommandInteraction, ComponentInteraction,
    ComponentInteractionDataKind, CreateAttachment, Http, Interaction, ModalInteraction,
};

use crate::adapter::DiscordAdapter;
use crate::interactions::{BridgeInteraction, BridgeReply, InteractionRouter};

/// Discord-Antwort-Typen (Interaction-Callback).
const CB_MESSAGE: u8 = 4;
const CB_DEFER: u8 = 5;
/// `UPDATE_MESSAGE` — editiert die Nachricht, an der die Komponente hängt
/// (nur für Button/Select-Interaktionen gültig).
const CB_UPDATE_MESSAGE: u8 = 7;
const CB_MODAL: u8 = 9;
const EPHEMERAL_FLAG: u64 = 64;

const DEFER_THRESHOLD: Duration = Duration::from_secs(2);

pub async fn dispatch(
    adapter: &Arc<DiscordAdapter>,
    router: &Arc<InteractionRouter>,
    interaction: &Interaction,
) {
    match interaction {
        Interaction::Command(cmd) => dispatch_command(adapter, router, cmd).await,
        Interaction::Component(component) => dispatch_component(adapter, router, component).await,
        Interaction::Modal(modal) => dispatch_modal(adapter, router, modal).await,
        _ => {}
    }
}

async fn dispatch_command(
    adapter: &Arc<DiscordAdapter>,
    router: &Arc<InteractionRouter>,
    cmd: &CommandInteraction,
) {
    let (qualified_name, options) = flatten_command(&cmd.data.name, &cmd.data.options);
    let Some(handler) = router.resolve_command(&qualified_name) else {
        tracing::warn!(command = %qualified_name, "Kein Handler für Slash-Command");
        return;
    };
    let bridge = BridgeInteraction {
        command: qualified_name,
        options,
        interaction_id: cmd.id.get(),
        user_id: cmd.user.id.get(),
        author_name: username(&cmd.user),
        author_display_name: interaction_display_name(cmd.member.as_deref(), &cmd.user),
        author_can_manage_roles: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_roles() || p.administrator())
            .unwrap_or(false),
        author_can_manage_guild: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_guild() || p.administrator())
            .unwrap_or(false),
        author_can_manage_channels: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_channels() || p.administrator())
            .unwrap_or(false),
        member_present: cmd.member.is_some(),
        guild_id: cmd.guild_id.map(|g| g.get()).unwrap_or(0),
        channel_id: cmd.channel_id.get(),
        ..BridgeInteraction::default()
    };
    respond(
        adapter,
        handler.handle(bridge),
        cmd.id.get(),
        &cmd.token,
        cmd.channel_id.get(),
        false, // Slash: kein UPDATE_MESSAGE
    )
    .await;
}

async fn dispatch_component(
    adapter: &Arc<DiscordAdapter>,
    router: &Arc<InteractionRouter>,
    component: &ComponentInteraction,
) {
    let custom_id = component.data.custom_id.to_string();
    let Some(handler) = router.resolve_component(&custom_id) else {
        // Unbekannte custom_ids sind okay — bis zum Voll-Cutover bedient
        // der Python-Bot seine eigenen Views.
        tracing::debug!(%custom_id, "Kein Handler für Komponente");
        return;
    };
    let values = match &component.data.kind {
        ComponentInteractionDataKind::StringSelect { values } => {
            values.iter().map(|v| v.to_string()).collect()
        }
        _ => Vec::new(),
    };
    let bridge = BridgeInteraction {
        custom_id,
        values,
        interaction_id: component.id.get(),
        user_id: component.user.id.get(),
        author_name: username(&component.user),
        author_display_name: interaction_display_name(component.member.as_ref(), &component.user),
        author_can_manage_roles: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_roles() || p.administrator())
            .unwrap_or(false),
        author_can_manage_guild: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_guild() || p.administrator())
            .unwrap_or(false),
        author_can_manage_channels: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_channels() || p.administrator())
            .unwrap_or(false),
        member_present: component.member.is_some(),
        guild_id: component.guild_id.map(|g| g.get()).unwrap_or(0),
        channel_id: component.channel_id.get(),
        message_id: Some(component.message.id.get()),
        ..BridgeInteraction::default()
    };
    respond(
        adapter,
        handler.handle(bridge),
        component.id.get(),
        &component.token,
        component.channel_id.get(),
        true, // Komponente: update_message erlaubt (in-place editieren)
    )
    .await;
}

async fn dispatch_modal(
    adapter: &Arc<DiscordAdapter>,
    router: &Arc<InteractionRouter>,
    modal: &ModalInteraction,
) {
    let custom_id = modal.data.custom_id.to_string();
    let Some(handler) = router.resolve_component(&custom_id) else {
        tracing::debug!(%custom_id, "Kein Handler für Modal");
        return;
    };
    // Eingabefelder: custom_id → Wert
    let mut options = std::collections::HashMap::new();
    for row in &modal.data.components {
        for component in &row.components {
            if let serenity::all::ActionRowComponent::InputText(input) = component {
                options.insert(
                    input.custom_id.to_string(),
                    json!(input.value.clone().unwrap_or_default()),
                );
            }
        }
    }
    let bridge = BridgeInteraction {
        custom_id,
        options,
        interaction_id: modal.id.get(),
        user_id: modal.user.id.get(),
        author_name: username(&modal.user),
        author_display_name: interaction_display_name(modal.member.as_ref(), &modal.user),
        author_can_manage_roles: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_roles() || p.administrator())
            .unwrap_or(false),
        author_can_manage_guild: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_guild() || p.administrator())
            .unwrap_or(false),
        author_can_manage_channels: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_channels() || p.administrator())
            .unwrap_or(false),
        member_present: modal.member.is_some(),
        guild_id: modal.guild_id.map(|g| g.get()).unwrap_or(0),
        channel_id: modal.channel_id.get(),
        message_id: modal.message.as_ref().map(|m| m.id.get()),
        ..BridgeInteraction::default()
    };
    respond(
        adapter,
        handler.handle(bridge),
        modal.id.get(),
        &modal.token,
        modal.channel_id.get(),
        false, // Modal-Submit: kein UPDATE_MESSAGE
    )
    .await;
}

fn username(user: &serenity::all::User) -> String {
    match user.discriminator {
        Some(d) => format!("{}#{:04}", user.name, d),
        None => user.name.to_string(),
    }
}

fn interaction_display_name(
    member: Option<&serenity::all::Member>,
    user: &serenity::all::User,
) -> String {
    display_name_from_parts(
        member.and_then(|m| m.nick.as_deref()),
        user.global_name.as_deref(),
        &user.name,
    )
}

fn display_name_from_parts(
    member_nick: Option<&str>,
    global_name: Option<&str>,
    username: &str,
) -> String {
    member_nick
        .filter(|name| !name.trim().is_empty())
        .or_else(|| global_name.filter(|name| !name.trim().is_empty()))
        .unwrap_or(username)
        .to_string()
}

/// Subcommands abflachen: ("steam", [links {…}]) → ("steam links", Optionen).
fn flatten_command(
    name: &str,
    options: &[CommandDataOption],
) -> (String, std::collections::HashMap<String, Value>) {
    if let Some(option) = options.first() {
        if let CommandDataOptionValue::SubCommand(inner) = &option.value {
            return (
                format!("{name} {}", option.name),
                inner.iter().map(option_to_pair).collect(),
            );
        }
        if let CommandDataOptionValue::SubCommandGroup(inner) = &option.value {
            let (sub_name, opts) = flatten_command(&option.name, inner);
            return (format!("{name} {sub_name}"), opts);
        }
    }
    (
        name.to_string(),
        options.iter().map(option_to_pair).collect(),
    )
}

fn option_to_pair(option: &CommandDataOption) -> (String, Value) {
    let value = match &option.value {
        CommandDataOptionValue::String(s) => json!(s),
        CommandDataOptionValue::Integer(i) => json!(i),
        CommandDataOptionValue::Number(n) => json!(n),
        CommandDataOptionValue::Boolean(b) => json!(b),
        CommandDataOptionValue::User(id) => json!(id.get()),
        CommandDataOptionValue::Channel(id) => json!(id.get()),
        CommandDataOptionValue::Role(id) => json!(id.get()),
        other => json!(format!("{other:?}")),
    };
    (option.name.to_string(), value)
}

fn message_data(reply: &BridgeReply) -> Value {
    let mut data = Map::new();
    if let Some(content) = &reply.content {
        data.insert("content".into(), json!(content));
    }
    if !reply.embeds.is_empty() {
        data.insert("embeds".into(), json!(reply.embeds));
    }
    if let Some(components) = &reply.components {
        data.insert("components".into(), components.clone());
    }
    if reply.ephemeral {
        data.insert("flags".into(), json!(EPHEMERAL_FLAG));
    }
    if !reply.attachments.is_empty() {
        // Discord ordnet die Multipart-Teile `files[i]` über diese id zu.
        let meta: Vec<Value> = reply
            .attachments
            .iter()
            .enumerate()
            .map(|(i, a)| json!({ "id": i, "filename": a.filename }))
            .collect();
        data.insert("attachments".into(), json!(meta));
    }
    Value::Object(data)
}

/// In-Memory-Anhänge → serenity-`CreateAttachment` (Multipart erledigt serenity).
fn build_files(reply: &BridgeReply) -> Vec<CreateAttachment> {
    reply
        .attachments
        .iter()
        .map(|a| CreateAttachment::bytes(a.data.clone(), a.filename.clone()))
        .collect()
}

fn modal_data(modal: &crate::interactions::ModalSpec) -> Value {
    json!({
        "custom_id": modal.custom_id,
        "title": modal.title,
        "components": modal.fields.iter().map(|field| json!({
            "type": 1,
            "components": [{
                "type": 4,
                "custom_id": field.custom_id,
                "label": field.label,
                "style": if field.paragraph { 2 } else { 1 },
                "placeholder": field.placeholder,
                "required": field.required,
                "min_length": field.min_length,
                "max_length": field.max_length,
            }],
        })).collect::<Vec<_>>(),
    })
}

/// Antwort senden — mit 2-Sekunden-Defer-Schwelle wie das Original.
async fn respond(
    adapter: &Arc<DiscordAdapter>,
    handler_future: impl std::future::Future<Output = BridgeReply>,
    interaction_id: u64,
    token: &str,
    channel_id: u64,
    allow_update: bool,
) {
    let http = adapter.http.clone();
    tokio::pin!(handler_future);

    let reply = match tokio::time::timeout(DEFER_THRESHOLD, &mut handler_future).await {
        Ok(reply) => {
            send_initial(
                adapter,
                &http,
                reply,
                interaction_id,
                token,
                channel_id,
                allow_update,
            )
            .await;
            return;
        }
        Err(_) => {
            // Defer (ephemeral) — Token bleibt 15 Minuten gültig
            let defer = json!({ "type": CB_DEFER, "data": { "flags": EPHEMERAL_FLAG } });
            if let Err(err) = http
                .create_interaction_response(interaction_id.into(), token, &defer, Vec::new())
                .await
            {
                tracing::warn!(%err, "Defer fehlgeschlagen");
            }
            handler_future.await
        }
    };

    // Followup nach Defer
    if let Some(channel_message) = &reply.channel_message {
        if let Some(message_id) = send_channel_panel(adapter, channel_id, channel_message).await {
            if let Some(hook) = &channel_message.response_message_hook {
                hook.on_response_message(message_id).await;
            }
        }
        let confirmation = json!({
            "content": channel_message.confirmation,
            "flags": EPHEMERAL_FLAG,
        });
        if let Err(err) = http
            .create_followup_message(token, &confirmation, Vec::new())
            .await
        {
            tracing::warn!(%err, "Followup (Bestätigung) fehlgeschlagen");
        }
        return;
    }
    if reply.modal.is_some() {
        tracing::warn!("Modal nach Defer nicht möglich — Handler-Design prüfen");
        return;
    }
    match http
        .create_followup_message(token, &message_data(&reply), build_files(&reply))
        .await
    {
        Ok(message) => run_response_hook(&reply, message.id.get()).await,
        Err(err) => tracing::warn!(%err, "Followup fehlgeschlagen"),
    }
}

async fn run_response_hook(reply: &BridgeReply, message_id: u64) {
    if let Some(hook) = &reply.response_message_hook {
        hook.on_response_message(message_id).await;
    }
}

async fn send_initial(
    adapter: &Arc<DiscordAdapter>,
    http: &Arc<Http>,
    reply: BridgeReply,
    interaction_id: u64,
    token: &str,
    channel_id: u64,
    allow_update: bool,
) {
    if let Some(modal) = &reply.modal {
        let response = json!({ "type": CB_MODAL, "data": modal_data(modal) });
        if let Err(err) = http
            .create_interaction_response(interaction_id.into(), token, &response, Vec::new())
            .await
        {
            tracing::warn!(%err, "Modal-Response fehlgeschlagen");
        }
        return;
    }
    if let Some(channel_message) = &reply.channel_message {
        if let Some(message_id) = send_channel_panel(adapter, channel_id, channel_message).await {
            if let Some(hook) = &channel_message.response_message_hook {
                hook.on_response_message(message_id).await;
            }
        }
        let response = json!({
            "type": CB_MESSAGE,
            "data": { "content": channel_message.confirmation, "flags": EPHEMERAL_FLAG },
        });
        if let Err(err) = http
            .create_interaction_response(interaction_id.into(), token, &response, Vec::new())
            .await
        {
            tracing::warn!(%err, "Panel-Bestätigung fehlgeschlagen");
        }
        return;
    }
    // Komponenten-Handler mit update_message → die bestehende Nachricht editieren.
    let cb = if reply.update_message && allow_update {
        CB_UPDATE_MESSAGE
    } else {
        CB_MESSAGE
    };
    let response = json!({ "type": cb, "data": message_data(&reply) });
    match http
        .create_interaction_response(interaction_id.into(), token, &response, build_files(&reply))
        .await
    {
        Ok(()) => {
            if reply.response_message_hook.is_some() {
                match http.get_original_interaction_response(token).await {
                    Ok(message) => run_response_hook(&reply, message.id.get()).await,
                    Err(err) => {
                        tracing::warn!(%err, "Interaction-Response-ID konnte nicht geladen werden");
                    }
                }
            }
        }
        Err(err) => tracing::warn!(%err, "Interaction-Response fehlgeschlagen"),
    }
}

async fn send_channel_panel(
    adapter: &Arc<DiscordAdapter>,
    channel_id: u64,
    panel: &crate::interactions::ChannelMessage,
) -> Option<u64> {
    let target_channel_id = panel.target_channel_id.unwrap_or(channel_id);
    let mut body = Map::new();
    if let Some(content) = &panel.content {
        body.insert("content".into(), json!(content));
    }
    if !panel.embeds.is_empty() {
        body.insert("embeds".into(), json!(panel.embeds));
    }
    if let Some(components) = &panel.components {
        body.insert("components".into(), components.clone());
    }
    if let Some(message_id) = panel.edit_message_id {
        match adapter
            .edit_raw_public(target_channel_id, message_id, &body)
            .await
        {
            Ok(()) => return Some(message_id),
            Err(err) => {
                let status = panel_edit_status_code(&err);
                tracing::warn!(%err, ?status, target_channel_id, message_id, "Panel-Edit fehlgeschlagen");
                if !should_repost_after_panel_edit_error(&err) {
                    return None;
                }
            }
        }
    }
    match adapter.send_raw_public(target_channel_id, &body).await {
        Ok(message_id) => Some(message_id),
        Err(err) => {
            tracing::warn!(%err, channel_id = target_channel_id, "Panel-Post fehlgeschlagen");
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelEditFailure {
    NotFound,
    Other,
}

fn classify_panel_edit_status(status: Option<u16>) -> PanelEditFailure {
    match status {
        Some(404) => PanelEditFailure::NotFound,
        _ => PanelEditFailure::Other,
    }
}

pub fn is_panel_edit_not_found_status(status: Option<u16>) -> bool {
    matches!(
        classify_panel_edit_status(status),
        PanelEditFailure::NotFound
    )
}

pub fn should_repost_after_panel_edit_error(err: &serenity::Error) -> bool {
    is_panel_edit_not_found_status(panel_edit_status_code(err))
}

pub fn panel_edit_status_code(err: &serenity::Error) -> Option<u16> {
    match err {
        serenity::Error::Http(http_err) => http_err.status_code().map(|status| status.as_u16()),
        _ => None,
    }
}

/// Slash-Commands syncen (Bulk-Overwrite). Guild-scoped wenn guild_id gesetzt.
pub async fn sync_commands(
    http: &Http,
    router: &InteractionRouter,
    guild_id: Option<u64>,
) -> Result<usize, serenity::Error> {
    let definitions = router.command_definitions();
    let count = definitions.len();
    match guild_id {
        Some(guild_id) => {
            http.create_guild_commands(guild_id.into(), &json!(definitions))
                .await?;
        }
        None => {
            http.create_global_commands(&json!(definitions)).await?;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_data_baut_flags_und_felder() {
        let reply = BridgeReply {
            content: Some("Hi".into()),
            embeds: vec![json!({"title": "T"})],
            components: Some(json!([{"type": 1, "components": []}])),
            ephemeral: true,
            ..BridgeReply::default()
        };
        let data = message_data(&reply);
        assert_eq!(data["content"], "Hi");
        assert_eq!(data["flags"], 64);
        assert_eq!(data["embeds"][0]["title"], "T");

        let public = BridgeReply {
            content: Some("Pub".into()),
            ..BridgeReply::default()
        };
        assert!(message_data(&public).get("flags").is_none());
    }

    #[test]
    fn modal_data_format() {
        let modal = crate::interactions::ModalSpec {
            custom_id: "m1".into(),
            title: "Titel".into(),
            fields: vec![crate::interactions::ModalField {
                custom_id: "f1".into(),
                label: "Feld".into(),
                placeholder: "…".into(),
                required: true,
                min_length: 1,
                max_length: 32,
                paragraph: false,
            }],
        };
        let data = modal_data(&modal);
        assert_eq!(data["custom_id"], "m1");
        assert_eq!(data["components"][0]["components"][0]["type"], 4);
        assert_eq!(data["components"][0]["components"][0]["max_length"], 32);
    }

    #[test]
    fn bridge_display_name_nutzt_nick_vor_global_name_vor_username() {
        assert_eq!(
            display_name_from_parts(Some("Server Nick"), Some("Global Name"), "username"),
            "Server Nick"
        );
        assert_eq!(
            display_name_from_parts(None, Some("Global Name"), "username"),
            "Global Name"
        );
        assert_eq!(display_name_from_parts(None, None, "username"), "username");
    }

    #[test]
    fn panel_edit_fallback_nur_bei_404() {
        assert_eq!(
            classify_panel_edit_status(Some(404)),
            PanelEditFailure::NotFound
        );
        assert_eq!(
            classify_panel_edit_status(Some(403)),
            PanelEditFailure::Other
        );
        assert_eq!(classify_panel_edit_status(None), PanelEditFailure::Other);
    }
}
