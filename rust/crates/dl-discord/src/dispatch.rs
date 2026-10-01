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
const DEFER_HTTP_TIMEOUT: Duration = Duration::from_secs(1);

fn aimod_needs_immediate_defer(custom_id: &str) -> bool {
    [
        "aimod:accept:",
        "aimod:ban:",
        "aimod:denysubmit:",
        "aimod:untimeout:",
        "aimod:unban:",
    ]
    .iter()
    .any(|prefix| custom_id.starts_with(prefix))
}

async fn await_handler_with_bounded_defer<Handler, Defer, Reply, Error>(
    handler: Handler,
    defer: Defer,
    defer_timeout: Duration,
) -> (
    Reply,
    Result<Result<(), Error>, tokio::time::error::Elapsed>,
)
where
    Handler: std::future::Future<Output = Reply>,
    Defer: std::future::Future<Output = Result<(), Error>>,
{
    tokio::join!(handler, tokio::time::timeout(defer_timeout, defer))
}

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

fn flatten_resolved_command(
    data: &serenity::all::CommandData,
) -> (String, std::collections::HashMap<String, Value>) {
    fn resolve(
        source: &[CommandDataOption],
        resolved: &serenity::all::CommandDataResolved,
        target: &mut std::collections::HashMap<String, Value>,
    ) {
        for option in source {
            match &option.value {
                CommandDataOptionValue::Attachment(id) => {
                    let value = resolved
                        .attachments
                        .get(id)
                        .map(|attachment| {
                            json!({
                                "url": attachment.url,
                                "content_type": attachment.content_type,
                                "size": attachment.size,
                                "filename": attachment.filename,
                            })
                        })
                        .unwrap_or(Value::Null);
                    target.insert(option.name.clone(), value);
                }
                CommandDataOptionValue::SubCommand(children)
                | CommandDataOptionValue::SubCommandGroup(children) => {
                    resolve(children, resolved, target)
                }
                _ => {}
            }
        }
    }
    let (name, mut options) = flatten_command(&data.name, &data.options);
    resolve(&data.options, &data.resolved, &mut options);
    (name, options)
}

async fn dispatch_command(
    adapter: &Arc<DiscordAdapter>,
    router: &Arc<InteractionRouter>,
    cmd: &CommandInteraction,
) {
    let (qualified_name, options) = flatten_resolved_command(&cmd.data);
    let Some(handler) = router.resolve_command(&qualified_name) else {
        tracing::warn!(command = %qualified_name, "Kein Handler für Slash-Command");
        return;
    };
    let bridge = BridgeInteraction {
        command: qualified_name,
        options,
        interaction_id: cmd.id.get(),
        user_id: cmd.user.id.get(),
        role_ids: interaction_role_ids(cmd.member.as_deref()),
        author_name: username(&cmd.user),
        author_display_name: interaction_display_name(cmd.member.as_deref(), &cmd.user),
        author_can_manage_roles: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_roles() || p.administrator())
            .unwrap_or(false),
        author_can_moderate_members: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.moderate_members() || p.administrator())
            .unwrap_or(false),
        author_can_ban_members: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.ban_members() || p.administrator())
            .unwrap_or(false),
        author_can_manage_guild: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_guild() || p.administrator())
            .unwrap_or(false),
        author_can_manage_messages: cmd
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_messages() || p.administrator())
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
        false,
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
        ComponentInteractionDataKind::UserSelect { values } => {
            values.iter().map(|id| id.get().to_string()).collect()
        }
        _ => Vec::new(),
    };
    let defer_immediately = aimod_needs_immediate_defer(&custom_id);
    let bridge = BridgeInteraction {
        custom_id,
        values,
        interaction_id: component.id.get(),
        user_id: component.user.id.get(),
        role_ids: interaction_role_ids(component.member.as_ref()),
        author_name: username(&component.user),
        author_display_name: interaction_display_name(component.member.as_ref(), &component.user),
        author_can_manage_roles: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_roles() || p.administrator())
            .unwrap_or(false),
        author_can_moderate_members: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.moderate_members() || p.administrator())
            .unwrap_or(false),
        author_can_ban_members: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.ban_members() || p.administrator())
            .unwrap_or(false),
        author_can_manage_guild: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_guild() || p.administrator())
            .unwrap_or(false),
        author_can_manage_messages: component
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_messages() || p.administrator())
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
        defer_immediately,
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
    let defer_immediately = aimod_needs_immediate_defer(&custom_id);
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
        role_ids: interaction_role_ids(modal.member.as_ref()),
        author_name: username(&modal.user),
        author_display_name: interaction_display_name(modal.member.as_ref(), &modal.user),
        author_can_manage_roles: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_roles() || p.administrator())
            .unwrap_or(false),
        author_can_moderate_members: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.moderate_members() || p.administrator())
            .unwrap_or(false),
        author_can_ban_members: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.ban_members() || p.administrator())
            .unwrap_or(false),
        author_can_manage_guild: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_guild() || p.administrator())
            .unwrap_or(false),
        author_can_manage_messages: modal
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.manage_messages() || p.administrator())
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
        defer_immediately,
    )
    .await;
}

fn username(user: &serenity::all::User) -> String {
    match user.discriminator {
        Some(d) => format!("{}#{:04}", user.name, d),
        None => user.name.to_string(),
    }
}

fn interaction_role_ids(member: Option<&serenity::all::Member>) -> Vec<u64> {
    member
        .map(|member| member.roles.iter().map(|role| role.get()).collect())
        .unwrap_or_default()
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
        CommandDataOptionValue::Attachment(id) => json!(id.get()),
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
    if let Some(allowed_mentions) = &reply.allowed_mentions {
        data.insert("allowed_mentions".into(), allowed_mentions.clone());
    }
    if let Some(flags) = reply.message_flags {
        data.insert("flags".into(), json!(flags));
    } else if reply.ephemeral {
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
        "components": modal.fields.iter().map(|field| {
            let mut input = serde_json::Map::from_iter([
                ("type".to_string(), json!(4)),
                ("custom_id".to_string(), json!(field.custom_id)),
                ("label".to_string(), json!(field.label)),
                (
                    "style".to_string(),
                    json!(if field.paragraph { 2 } else { 1 }),
                ),
                ("placeholder".to_string(), json!(field.placeholder)),
                ("required".to_string(), json!(field.required)),
                ("min_length".to_string(), json!(field.min_length)),
                ("max_length".to_string(), json!(field.max_length)),
            ]);
            if let Some(value) = &field.value {
                input.insert("value".to_string(), json!(value));
            }
            json!({
                "type": 1,
                "components": [input],
            })
        }).collect::<Vec<_>>(),
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
    defer_immediately: bool,
) {
    let http = adapter.http.clone();
    tokio::pin!(handler_future);

    let reply = if defer_immediately {
        // Moderationsaktionen koennen Discord-HTTP und DB-Schreibvorgaenge ausloesen.
        // Deshalb zuerst bestaetigen und erst danach den Handler starten. So konkurriert
        // der Interaction-ACK nicht mit der eigentlichen Moderationsaktion um Zeit.
        let defer = json!({ "type": CB_DEFER, "data": { "flags": EPHEMERAL_FLAG } });
        let defer_result = tokio::time::timeout(
            DEFER_HTTP_TIMEOUT,
            http.create_interaction_response(interaction_id.into(), token, &defer, Vec::new()),
        )
        .await;
        match defer_result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::warn!(%err, "Sofort-Defer fehlgeschlagen"),
            Err(_) => tracing::warn!(
                timeout_ms = DEFER_HTTP_TIMEOUT.as_millis(),
                "Sofort-Defer-HTTP hat Zeitlimit ueberschritten"
            ),
        }
        handler_future.await
    } else {
        match tokio::time::timeout(DEFER_THRESHOLD, &mut handler_future).await {
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
                let (reply, defer_result) = await_handler_with_bounded_defer(
                    &mut handler_future,
                    http.create_interaction_response(
                        interaction_id.into(),
                        token,
                        &defer,
                        Vec::new(),
                    ),
                    DEFER_HTTP_TIMEOUT,
                )
                .await;
                match defer_result {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => tracing::warn!(%err, "Defer fehlgeschlagen"),
                    Err(_) => tracing::warn!(
                        timeout_ms = DEFER_HTTP_TIMEOUT.as_millis(),
                        "Defer-HTTP hat Zeitlimit ueberschritten"
                    ),
                }
                reply
            }
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
        Err(err) => {
            tracing::warn!(%err, "Followup fehlgeschlagen");
            if let Some(fallback) = &reply.fallback {
                if let Err(fallback_err) = http
                    .create_followup_message(token, &message_data(fallback), build_files(fallback))
                    .await
                {
                    tracing::warn!(%fallback_err, "Fallback-Followup fehlgeschlagen");
                }
            }
        }
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
        Err(err) => {
            tracing::warn!(%err, "Interaction-Response fehlgeschlagen");
            if let Some(fallback) = &reply.fallback {
                let fallback_response = json!({ "type": cb, "data": message_data(fallback) });
                if let Err(fallback_err) = http
                    .create_interaction_response(
                        interaction_id.into(),
                        token,
                        &fallback_response,
                        build_files(fallback),
                    )
                    .await
                {
                    tracing::warn!(%fallback_err, "Fallback-Interaction-Response fehlgeschlagen");
                }
            }
        }
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
    #[test]
    fn attachment_option_is_resolved_from_discord_data() {
        let data: serenity::all::CommandData = serde_json::from_value(serde_json::json!({
            "id":"10", "name":"brain", "type":1,
            "options":[{"name":"frage","type":3,"value":"Was siehst du?"},{"name":"bild","type":11,"value":"20"}],
            "resolved":{"attachments":{"20":{"id":"20","filename":"screenshot.png","size":1234,"content_type":"image/png","url":"https://cdn.discordapp.com/attachments/1/20/screenshot.png","proxy_url":"https://media.discordapp.net/attachments/1/20/screenshot.png"}}}
        })).expect("gültige Discord-Testdaten");
        let (name, options) = super::flatten_resolved_command(&data);
        assert_eq!(name, "brain");
        assert_eq!(options["frage"], "Was siehst du?");
        assert_eq!(options["bild"]["size"], 1234);
        assert_eq!(options["bild"]["content_type"], "image/png");
    }

    #[test]
    fn unresolved_attachment_is_present_but_invalid_not_silently_omitted() {
        let data: serenity::all::CommandData = serde_json::from_value(serde_json::json!({
            "id":"10", "name":"brain", "type":1,
            "options":[{"name":"bild","type":11,"value":"20"}]
        }))
        .expect("gültige Discord-Testdaten ohne Auflösung");
        let (_, options) = super::flatten_resolved_command(&data);
        assert!(options.contains_key("bild"));
        assert!(options["bild"].is_null());
    }
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn message_data_baut_flags_und_felder() {
        let reply = BridgeReply {
            content: Some("Hi".into()),
            embeds: vec![json!({"title": "T"})],
            components: Some(json!([{"type": 1, "components": []}])),
            ephemeral: true,
            allowed_mentions: Some(json!({"parse": []})),
            ..BridgeReply::default()
        };
        let data = message_data(&reply);
        assert_eq!(data["content"], "Hi");
        assert_eq!(data["flags"], 64);
        assert_eq!(data["embeds"][0]["title"], "T");
        assert_eq!(data["allowed_mentions"]["parse"], json!([]));

        let v2 = BridgeReply {
            message_flags: Some(64 | (1 << 15)),
            ..BridgeReply::default()
        };
        assert_eq!(message_data(&v2)["flags"], 64 | (1 << 15));

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
                value: Some("Vorbelegt".into()),
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
        assert_eq!(data["components"][0]["components"][0]["value"], "Vorbelegt");
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
    fn langsame_aimod_aktionen_deferen_vor_dem_handler() {
        for custom_id in [
            "aimod:accept:case-1",
            "aimod:ban:case-1",
            "aimod:denysubmit:case-1",
            "aimod:untimeout:case-1",
            "aimod:unban:case-1",
        ] {
            assert!(aimod_needs_immediate_defer(custom_id), "{custom_id}");
        }
        assert!(!aimod_needs_immediate_defer("aimod:deny:case-1"));
        assert!(!aimod_needs_immediate_defer("other:accept:case-1"));
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

    #[tokio::test]
    async fn defer_http_blockiert_den_laufenden_handler_nicht() {
        let handler_done = Arc::new(tokio::sync::Notify::new());
        let handler_done_signal = handler_done.clone();
        let defer_release = Arc::new(tokio::sync::Notify::new());
        let defer_release_waiter = defer_release.clone();
        let task = tokio::spawn(async move {
            await_handler_with_bounded_defer(
                async move {
                    handler_done_signal.notify_one();
                    42_u64
                },
                async move {
                    defer_release_waiter.notified().await;
                    Ok::<(), &'static str>(())
                },
                Duration::from_secs(5),
            )
            .await
        });

        tokio::time::timeout(Duration::from_millis(250), handler_done.notified())
            .await
            .expect("Handler muss waehrend des Defer-HTTP weitergepollt werden");
        defer_release.notify_one();
        let (reply, defer_result) = task.await.expect("orchestration task");
        assert_eq!(reply, 42);
        assert!(matches!(defer_result, Ok(Ok(()))));
    }

    #[tokio::test]
    async fn haengendes_defer_http_ist_hart_begrenzt() {
        let handler_finished = Arc::new(AtomicBool::new(false));
        let handler_finished_signal = handler_finished.clone();
        let completed = tokio::time::timeout(
            Duration::from_millis(250),
            await_handler_with_bounded_defer(
                async move {
                    handler_finished_signal.store(true, Ordering::SeqCst);
                    7_u64
                },
                std::future::pending::<Result<(), &'static str>>(),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("Defer-HTTP darf nicht unbegrenzt haengen");

        assert_eq!(completed.0, 7);
        assert!(completed.1.is_err(), "Defer muss als Timeout enden");
        assert!(handler_finished.load(Ordering::SeqCst));
    }
}
