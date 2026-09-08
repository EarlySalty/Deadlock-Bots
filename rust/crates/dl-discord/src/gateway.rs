//! Gateway-Aufbau (serenity-Client). Start ist user-gated: Bis zum Cutover
//! hält der Python-Bot die Discord-Session; zwei aktive Handler würden
//! Events doppelt verarbeiten.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serenity::all::{
    CommandDataOption, CommandDataOptionValue, Context, EventHandler, GatewayIntents, GuildChannel,
    GuildId, GuildMemberFlags, GuildMemberUpdateEvent, Interaction, InviteCreateEvent,
    InviteDeleteEvent, Member, Message, MessageType, OnlineStatus, Permissions, Reaction,
    ReactionType, Ready, User, UserId, VoiceState,
};
use serenity::async_trait;
use serenity::gateway::ActivityData;
use songbird::{SerenityInit, Songbird};

use crate::adapter::DiscordAdapter;
use crate::core_user_sync::{
    profile_from_interaction, profile_from_user, CoreUserEventKind, CoreUserProfile, CoreUserSync,
};
use crate::dispatcher::{
    member_screening_completed_event, role_events_from_diff, ChannelEvent, Dispatcher,
    GatewayEvent, InteractionEvent, MemberEvent, MessageAttachment, MessageEvent, PresenceEvent,
    VoiceEvent,
};
use crate::interactions::InteractionRouter;
use crate::invite_tracker::InviteTracker;

const IMAGE_ATTACHMENT_EXTENSIONS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif"];

// Discord-Systemmeldungen tragen teilweise einen menschlichen Autor (z. B.
// MemberJoin). Sie sind keine Nachricht dieses Mitglieds. Inhalt darf leer sein:
// Sticker, Anhänge und Weiterleitungen bleiben reguläre Nutzernachrichten.
fn is_user_message(message: &Message) -> bool {
    !message.author.bot
        && matches!(
            message.kind,
            MessageType::Regular | MessageType::InlineReply
        )
}

fn ready_contains_guild(guild_ids: impl IntoIterator<Item = u64>, guild_id: u64) -> bool {
    guild_ids.into_iter().any(|candidate| candidate == guild_id)
}

fn voice_worker_intents() -> GatewayIntents {
    GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES
}

struct Handler {
    adapter: Arc<DiscordAdapter>,
    dispatcher: Arc<Dispatcher>,
    router: Arc<InteractionRouter>,
    invite_tracker: Arc<InviteTracker>,
    core_user_sync: Arc<CoreUserSync>,
    reaction_roles: Option<Arc<dyn ReactionRoleGatewayPort>>,
    feature_module_count: usize,
    command_prefix: String,
    recording_readiness: Arc<AtomicBool>,
    recording_guild_id: u64,
}

struct VoiceWorkerHandler {
    readiness: Arc<AtomicBool>,
    guild_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PermissionFlags {
    author_is_admin: bool,
    author_can_manage_messages: bool,
    author_can_manage_guild: bool,
    author_is_staff: bool,
}

fn is_staff_permissions(perms: Permissions) -> bool {
    perms.administrator() || perms.manage_messages() || perms.manage_guild()
}

fn permission_flags(perms: Permissions) -> PermissionFlags {
    PermissionFlags {
        author_is_admin: perms.administrator(),
        author_can_manage_messages: perms.manage_messages(),
        author_can_manage_guild: perms.administrator() || perms.manage_guild(),
        author_is_staff: is_staff_permissions(perms),
    }
}

fn is_image_attachment(content_type: Option<&str>, filename: &str) -> bool {
    let content_type_is_image = content_type
        .map(|c| c.to_ascii_lowercase().starts_with("image/"))
        .unwrap_or(false);
    let filename = filename.to_ascii_lowercase();
    content_type_is_image
        || IMAGE_ATTACHMENT_EXTENSIONS
            .iter()
            .any(|ext| filename.ends_with(ext))
}

pub fn presence_activity_name(feature_module_count: usize, command_prefix: &str) -> String {
    format!("{feature_module_count} Cogs | {command_prefix}help")
}

fn screening_completed_from_member_update(
    guild_id: u64,
    user_id: u64,
    before_pending: bool,
    after_pending: Option<bool>,
) -> Option<MemberEvent> {
    // Serenity 0.12.5 modelliert `pending` als `bool` mit `#[serde(default)]`;
    // Feld-Präsenz aus dem Gateway-Payload ist hier nicht mehr rekonstruierbar.
    // Deshalb nie auf `event.pending` zurückfallen. Restrisiko: Wenn Serenity
    // den Cache bereits mit einem defaulteten `false` überschrieben hat, bleibt
    // nur die Auto-Start-Guard-Schicht in `dl-community` als Schadensbegrenzung.
    let after_pending = after_pending?;
    member_screening_completed_event(guild_id, user_id, before_pending, after_pending)
}

fn completed_onboarding_from_member_update(
    guild_id: u64,
    user_id: u64,
    before_flags: GuildMemberFlags,
    after_flags: Option<GuildMemberFlags>,
) -> Option<MemberEvent> {
    let after_flags = after_flags?;
    (!before_flags.contains(GuildMemberFlags::COMPLETED_ONBOARDING)
        && after_flags.contains(GuildMemberFlags::COMPLETED_ONBOARDING))
    .then_some(MemberEvent::NativeOnboardingCompleted { guild_id, user_id })
}

fn is_self_reaction_user(
    user_id: UserId,
    current_user_id: UserId,
    application_id: Option<u64>,
) -> bool {
    user_id == current_user_id || application_id == Some(user_id.get())
}

fn command_route(name: &str, options: &[CommandDataOption]) -> String {
    if let Some(option) = options.first() {
        if let CommandDataOptionValue::SubCommand(inner) = &option.value {
            return format!("{name} {}", command_route(&option.name, inner));
        }
        if let CommandDataOptionValue::SubCommandGroup(inner) = &option.value {
            return format!("{name} {}", command_route(&option.name, inner));
        }
    }
    name.to_string()
}

fn interaction_event(interaction: &Interaction) -> Option<InteractionEvent> {
    match interaction {
        Interaction::Command(cmd) => Some(InteractionEvent {
            guild_id: cmd.guild_id.map(|id| id.get()),
            channel_id: Some(cmd.channel_id.get()),
            message_id: None,
            interaction_id: cmd.id.get(),
            user_id: cmd.user.id.get(),
            interaction_kind: "command",
            route: Some(command_route(&cmd.data.name, &cmd.data.options)),
            occurred_at: cmd.id.created_at().unix_timestamp(),
        }),
        Interaction::Component(component) => Some(InteractionEvent {
            guild_id: component.guild_id.map(|id| id.get()),
            channel_id: Some(component.channel_id.get()),
            message_id: Some(component.message.id.get()),
            interaction_id: component.id.get(),
            user_id: component.user.id.get(),
            interaction_kind: "component",
            route: Some(component.data.custom_id.to_string()),
            occurred_at: component.id.created_at().unix_timestamp(),
        }),
        Interaction::Modal(modal) => Some(InteractionEvent {
            guild_id: modal.guild_id.map(|id| id.get()),
            channel_id: Some(modal.channel_id.get()),
            message_id: modal.message.as_ref().map(|message| message.id.get()),
            interaction_id: modal.id.get(),
            user_id: modal.user.id.get(),
            interaction_kind: "modal",
            route: Some(modal.data.custom_id.to_string()),
            occurred_at: modal.id.created_at().unix_timestamp(),
        }),
        _ => None,
    }
}

#[async_trait]
pub trait ReactionRoleGatewayPort: Send + Sync {
    async fn reaction_add(&self, event: ReactionRoleAddEvent);

    async fn reaction_remove(
        &self,
        guild_id: u64,
        channel_id: u64,
        message_id: u64,
        user_id: u64,
        emoji: ReactionType,
        is_bot: bool,
    );
}

#[derive(Debug, Clone)]
pub struct ReactionRoleAddEvent {
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub user_id: u64,
    pub display_name: Option<String>,
    pub emoji: ReactionType,
    pub is_bot: bool,
}

async fn reaction_user_is_bot(
    ctx: &Context,
    guild_id: Option<GuildId>,
    user_id: UserId,
    member_is_bot: Option<bool>,
) -> bool {
    if is_self_reaction_user(
        user_id,
        ctx.cache.current_user().id,
        ctx.http.application_id().map(|id| id.get()),
    ) {
        return true;
    }
    if let Some(is_bot) = member_is_bot {
        return is_bot;
    }
    if let Some(guild_id) = guild_id {
        let cached = ctx
            .cache
            .guild(guild_id)
            .and_then(|guild| guild.members.get(&user_id).map(|member| member.user.bot));
        if let Some(is_bot) = cached {
            return is_bot;
        }
    }
    match ctx.http.get_user(user_id).await {
        Ok(user) => user.bot,
        Err(err) => {
            tracing::debug!(%err, user_id = user_id.get(), "Reaction-User nicht auflösbar; Event wird verarbeitet");
            false
        }
    }
}

impl Handler {
    fn record_core_user(&self, kind: CoreUserEventKind, profile: Option<CoreUserProfile>) {
        let Some(profile) = profile else {
            return;
        };
        self.core_user_sync.record(kind, profile);
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        self.adapter.gateway_ready.store(true, Ordering::Relaxed);
        self.recording_readiness.store(
            ready_contains_guild(
                ready.guilds.iter().map(|guild| guild.id.get()),
                self.recording_guild_id,
            ),
            Ordering::Release,
        );
        let activity_name = presence_activity_name(self.feature_module_count, &self.command_prefix);
        ctx.set_activity(Some(ActivityData::watching(activity_name)));
        self.dispatcher.publish_gateway(GatewayEvent::Ready {
            guild_count: ready.guilds.len(),
        });
        tracing::info!(user = %ready.user.name, guilds = ready.guilds.len(), "Gateway READY");
    }

    async fn cache_ready(&self, ctx: Context, guilds: Vec<GuildId>) {
        self.dispatcher.publish_gateway(GatewayEvent::CacheReady {
            guild_ids: guilds.iter().map(|gid| gid.get()).collect(),
        });
        // Invite-Snapshots primen, damit der erste Join nach Start klassifiziert
        // werden kann (sonst „baseline_missing"). Joins treffen erst nach READY ein.
        for gid in &guilds {
            self.invite_tracker.prime(&ctx.http, gid.get()).await;
        }
        tracing::info!(guilds = guilds.len(), "Invite-Snapshots geprimt");
    }

    async fn presence_update(&self, _ctx: Context, new_data: serenity::all::Presence) {
        let Some(guild_id) = new_data.guild_id else {
            return;
        };
        if matches!(
            new_data.status,
            OnlineStatus::Offline | OnlineStatus::Invisible
        ) {
            return;
        }
        self.dispatcher.publish_presence(PresenceEvent {
            guild_id: guild_id.get(),
            user_id: new_data.user.id.get(),
        });
    }

    async fn interaction_create(&self, _ctx: Context, interaction: serenity::all::Interaction) {
        self.record_core_user(
            CoreUserEventKind::InteractionCreate,
            profile_from_interaction(&interaction),
        );
        if let Some(event) = interaction_event(&interaction) {
            self.dispatcher.publish_interaction(event);
        }
        crate::dispatch::dispatch(&self.adapter, &self.router, &interaction).await;
    }

    async fn reaction_add(&self, ctx: Context, add: Reaction) {
        let Some(port) = &self.reaction_roles else {
            return;
        };
        let (Some(guild_id), Some(user_id)) = (add.guild_id, add.user_id) else {
            return;
        };
        let member_is_bot = add.member.as_ref().map(|member| member.user.bot);
        let display_name = add
            .member
            .as_ref()
            .map(|member| member.display_name().to_string());
        let is_bot = reaction_user_is_bot(&ctx, add.guild_id, user_id, member_is_bot).await;
        port.reaction_add(ReactionRoleAddEvent {
            guild_id: guild_id.get(),
            channel_id: add.channel_id.get(),
            message_id: add.message_id.get(),
            user_id: user_id.get(),
            display_name,
            emoji: add.emoji,
            is_bot,
        })
        .await;
    }

    async fn reaction_remove(&self, ctx: Context, removed: Reaction) {
        let Some(port) = &self.reaction_roles else {
            return;
        };
        let (Some(guild_id), Some(user_id)) = (removed.guild_id, removed.user_id) else {
            return;
        };
        let is_bot = reaction_user_is_bot(&ctx, removed.guild_id, user_id, None).await;
        port.reaction_remove(
            guild_id.get(),
            removed.channel_id.get(),
            removed.message_id.get(),
            user_id.get(),
            removed.emoji,
            is_bot,
        )
        .await;
    }

    async fn message(&self, ctx: Context, message: Message) {
        if !is_user_message(&message) {
            return;
        }
        self.record_core_user(
            CoreUserEventKind::MessageCreate,
            profile_from_user(&message.author),
        );
        // Admin-Flag bleibt eng fuer Admin-Commands; Staff-Schutz fuer
        // Moderation folgt Python: administrator || manage_messages || manage_guild.
        let (
            author_is_admin,
            author_can_manage_messages,
            author_can_manage_guild,
            author_is_staff,
            author_joined_at,
            author_staff_status_known,
        ) = message
            .guild_id
            .and_then(|guild_id| {
                let guild = ctx.cache.guild(guild_id)?;
                if let Some(member) = guild.members.get(&message.author.id) {
                    let flags = permission_flags(guild.member_permissions(member));
                    return Some((
                        flags.author_is_admin,
                        flags.author_can_manage_messages,
                        flags.author_can_manage_guild,
                        flags.author_is_staff,
                        member.joined_at.map(|t| t.unix_timestamp()),
                        true,
                    ));
                }
                let member = message.member.as_deref()?;
                let flags =
                    permission_flags(guild.partial_member_permissions(message.author.id, member));
                Some((
                    flags.author_is_admin,
                    flags.author_can_manage_messages,
                    flags.author_can_manage_guild,
                    flags.author_is_staff,
                    member.joined_at.map(|t| t.unix_timestamp()),
                    true,
                ))
            })
            .unwrap_or((false, false, false, false, None, false));
        let image_attachment_urls: Vec<String> = message
            .attachments
            .iter()
            .filter(|a| is_image_attachment(a.content_type.as_deref(), &a.filename))
            .map(|a| a.url.clone())
            .collect();
        let attachments: Vec<MessageAttachment> = message
            .attachments
            .iter()
            .map(|attachment| MessageAttachment {
                url: attachment.url.clone(),
                content_type: attachment.content_type.clone().unwrap_or_default(),
                filename: attachment.filename.clone(),
            })
            .collect();
        let image_attachment_count = image_attachment_urls.len() as u32;
        let reply_message_id = message
            .message_reference
            .as_ref()
            .and_then(|reference| reference.message_id)
            .map(|id| id.get());
        let reply_channel_id = message
            .message_reference
            .as_ref()
            .map(|reference| reference.channel_id.get());
        self.dispatcher.publish_message(MessageEvent {
            guild_id: message.guild_id.map(|g| g.get()),
            channel_id: message.channel_id.get(),
            message_id: message.id.get(),
            author_id: message.author.id.get(),
            author_display_name: message
                .author_nick(&ctx)
                .await
                .unwrap_or_else(|| message.author.name.to_string()),
            author_is_admin,
            author_can_manage_messages,
            author_can_manage_guild,
            author_is_staff,
            author_staff_status_known,
            content: message.content.clone(),
            message_created_at: message.timestamp.unix_timestamp(),
            is_reply: message.message_reference.is_some(),
            reply_message_id,
            reply_channel_id,
            attachment_count: message.attachments.len() as u32,
            image_attachment_count,
            image_attachment_urls,
            attachments,
            author_created_at: message.author.id.created_at().unix_timestamp(),
            author_joined_at,
        });
    }

    async fn guild_member_addition(&self, ctx: Context, member: Member) {
        self.record_core_user(
            CoreUserEventKind::GuildMemberAdd,
            profile_from_user(&member.user),
        );
        // Beitrittsquelle per Invite-uses-Delta erkennen (rohe Metadaten).
        let metadata = self.invite_tracker.on_join(&ctx.http, &member).await;
        let join_position = ctx
            .cache
            .guild(member.guild_id)
            .map(|g| g.members.len() as i64);
        self.dispatcher.publish_member(MemberEvent::Join {
            guild_id: member.guild_id.get(),
            user_id: member.user.id.get(),
            display_name: member.display_name().to_string(),
            account_created_at: member.user.id.created_at().unix_timestamp(),
            join_position,
            is_bot: member.user.bot,
            metadata,
        });
    }

    async fn guild_member_removal(
        &self,
        _ctx: Context,
        guild_id: GuildId,
        user: User,
        member: Option<Member>,
    ) {
        self.dispatcher.publish_member(MemberEvent::Remove {
            guild_id: guild_id.get(),
            user_id: user.id.get(),
            display_name: member
                .as_ref()
                .map(|m| m.display_name().to_string())
                .or_else(|| user.global_name.clone())
                .unwrap_or_else(|| user.name.to_string()),
            is_bot: user.bot,
        });
    }

    async fn guild_member_update(
        &self,
        _ctx: Context,
        old: Option<Member>,
        new: Option<Member>,
        event: GuildMemberUpdateEvent,
    ) {
        self.record_core_user(
            CoreUserEventKind::GuildMemberUpdate,
            profile_from_user(&event.user),
        );
        let guild_id = event.guild_id.get();
        let user_id = event.user.id.get();

        // Rollen und Member-Screening diffen. Vorher-Zustand kommt aus dem
        // Cache (old); ohne Cache kein sicherer Übergang möglich.
        let Some(old) = old else {
            return;
        };
        if let Some(event) =
            completed_onboarding_from_member_update(guild_id, user_id, old.flags, event.flags)
        {
            self.dispatcher.publish_member(event);
        }
        if let Some(event) = screening_completed_from_member_update(
            guild_id,
            user_id,
            old.pending,
            new.as_ref().map(|m| m.pending),
        ) {
            self.dispatcher.publish_member(event);
        }
        let after_roles: &[serenity::all::RoleId] = new
            .as_ref()
            .map(|m| m.roles.as_slice())
            .unwrap_or(&event.roles);
        let before_roles: Vec<u64> = old.roles.iter().map(|role| role.get()).collect();
        let after_roles: Vec<u64> = after_roles.iter().map(|role| role.get()).collect();
        for event in role_events_from_diff(guild_id, user_id, &before_roles, &after_roles) {
            self.dispatcher.publish_role(event);
        }
    }

    async fn guild_ban_addition(&self, _ctx: Context, guild_id: GuildId, banned_user: User) {
        self.dispatcher.publish_member(MemberEvent::Ban {
            guild_id: guild_id.get(),
            user_id: banned_user.id.get(),
            display_name: banned_user.name.to_string(),
            is_bot: banned_user.bot,
        });
    }

    async fn guild_ban_removal(&self, _ctx: Context, guild_id: GuildId, unbanned_user: User) {
        self.dispatcher.publish_member(MemberEvent::Unban {
            guild_id: guild_id.get(),
            user_id: unbanned_user.id.get(),
            display_name: unbanned_user.name.to_string(),
            is_bot: unbanned_user.bot,
        });
    }

    async fn invite_create(&self, _ctx: Context, data: InviteCreateEvent) {
        self.invite_tracker.on_invite_create(&data).await;
    }

    async fn invite_delete(&self, _ctx: Context, data: InviteDeleteEvent) {
        if let Some(guild_id) = data.guild_id {
            self.invite_tracker
                .on_invite_delete(guild_id.get(), &data.code)
                .await;
        }
    }

    async fn voice_state_update(&self, _ctx: Context, old: Option<VoiceState>, new: VoiceState) {
        let Some(guild_id) = new.guild_id.map(|g| g.get()) else {
            return;
        };
        let user_id = new.user_id.get();
        let old_channel = old.as_ref().and_then(|v| v.channel_id).map(|c| c.get());
        let new_channel = new.channel_id.map(|c| c.get());
        let event = match (old_channel, new_channel) {
            (None, Some(channel_id)) => VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id,
            },
            (Some(channel_id), None) => VoiceEvent::Leave {
                guild_id,
                user_id,
                channel_id,
            },
            (Some(from), Some(to)) if from != to => VoiceEvent::Move {
                guild_id,
                user_id,
                from_channel_id: from,
                to_channel_id: to,
            },
            (Some(channel_id), Some(_)) => {
                let muted = |vs: &VoiceState| vs.mute || vs.deaf || vs.self_mute || vs.self_deaf;
                VoiceEvent::Update {
                    guild_id,
                    user_id,
                    channel_id,
                    was_muted: old.as_ref().map(&muted).unwrap_or(false),
                    is_muted: muted(&new),
                }
            }
            (None, None) => return,
        };
        self.dispatcher.publish_voice(event);
    }

    async fn channel_update(&self, _ctx: Context, old: Option<GuildChannel>, new: GuildChannel) {
        if new.kind != serenity::all::ChannelType::Voice {
            return;
        }
        let before = old
            .as_ref()
            .filter(|channel| channel.kind == serenity::all::ChannelType::Voice)
            .and_then(|channel| channel.parent_id.map(|parent| parent.get()));
        let after = new.parent_id.map(|parent| parent.get());
        if before != after {
            self.dispatcher
                .publish_channel(ChannelEvent::VoiceCategoryChanged {
                    guild_id: new.guild_id.get(),
                    channel_id: new.id.get(),
                    before_category_id: before,
                    after_category_id: after,
                });
        }
        if old
            .as_ref()
            .is_some_and(|channel| channel.name != new.name || before != after)
        {
            self.dispatcher
                .publish_channel(ChannelEvent::VoiceChannelUpdated {
                    guild_id: new.guild_id.get(),
                    channel_id: new.id.get(),
                });
        }
    }
}

#[async_trait]
impl EventHandler for VoiceWorkerHandler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        let ready_for_guild = ready_contains_guild(
            ready.guilds.iter().map(|guild| guild.id.get()),
            self.guild_id,
        );
        self.readiness.store(ready_for_guild, Ordering::Release);
        tracing::info!(
            guild_id = self.guild_id,
            ready = ready_for_guild,
            "Scrim-Record: Voice-Worker READY"
        );
    }
}

pub struct GatewayClientOptions {
    pub reaction_roles: Option<Arc<dyn ReactionRoleGatewayPort>>,
    pub pool: sqlx::PgPool,
    pub feature_module_count: usize,
    pub command_prefix: String,
    pub enable_presence_intent: bool,
    pub recording_readiness: Arc<AtomicBool>,
    pub recording_guild_id: u64,
}

/// Baut den serenity-Client. Aufrufer entscheidet über den Start
/// (DL_BOT_GATEWAY=1) und besitzt die Laufzeit-Task.
pub async fn build_client(
    token: &str,
    adapter: Arc<DiscordAdapter>,
    dispatcher: Arc<Dispatcher>,
    router: Arc<InteractionRouter>,
    options: GatewayClientOptions,
    songbird_manager: Arc<Songbird>,
) -> serenity::Result<serenity::Client> {
    let mut intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::GUILD_INVITES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    if options.enable_presence_intent {
        intents |= GatewayIntents::GUILD_PRESENCES;
    }
    serenity::Client::builder(token, intents)
        .register_songbird_with(songbird_manager)
        .event_handler(Handler {
            adapter,
            dispatcher,
            router,
            invite_tracker: Arc::new(InviteTracker::new(options.pool.clone())),
            core_user_sync: Arc::new(CoreUserSync::new(options.pool)),
            reaction_roles: options.reaction_roles,
            feature_module_count: options.feature_module_count,
            command_prefix: options.command_prefix,
            recording_readiness: options.recording_readiness,
            recording_guild_id: options.recording_guild_id,
        })
        .await
}

/// Baut die zweite, auf Voice-Aufnahme beschränkte Gateway-Identität.
pub async fn build_voice_worker_client(
    token: &str,
    manager: Arc<Songbird>,
    readiness: Arc<AtomicBool>,
    guild_id: u64,
) -> serenity::Result<serenity::Client> {
    serenity::Client::builder(token, voice_worker_intents())
        .register_songbird_with(manager)
        .event_handler(VoiceWorkerHandler {
            readiness,
            guild_id,
        })
        .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn member_join_mit_menschlichem_autor_ist_keine_chatnachricht() {
        let mut message = serenity::all::Message::default();
        message.author.bot = false;
        message.kind = serenity::all::MessageType::MemberJoin;
        assert!(!super::is_user_message(&message));
        message.kind = serenity::all::MessageType::PinsAdd;
        assert!(!super::is_user_message(&message));
    }

    #[test]
    fn echte_nachrichten_bleiben_auch_ohne_text_erhalten() {
        let mut message = serenity::all::Message::default();
        for kind in [
            serenity::all::MessageType::Regular,
            serenity::all::MessageType::InlineReply,
        ] {
            message.kind = kind;
            message.content.clear(); // Anhänge, Sticker und Weiterleitungen benötigen keinen Text.
            assert!(super::is_user_message(&message));
            message.content = "Hallo zusammen".into();
            assert!(super::is_user_message(&message));
        }
        message.author.bot = true;
        assert!(!super::is_user_message(&message));
    }
    use super::*;

    #[test]
    fn ready_presence_name_matches_python_style() {
        assert_eq!(presence_activity_name(47, "!"), "47 Cogs | !help");
    }

    #[test]
    fn recording_readiness_requires_the_fixed_guild_in_ready() {
        assert!(ready_contains_guild([7, 11, 13], 11));
        assert!(!ready_contains_guild([7, 13], 11));
        assert!(!ready_contains_guild([], 11));
    }

    #[test]
    fn voice_worker_uses_only_guild_and_voice_state_intents() {
        assert_eq!(
            voice_worker_intents(),
            GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES
        );
    }

    #[test]
    fn staff_permissions_match_python_guard_skip() {
        assert!(is_staff_permissions(Permissions::ADMINISTRATOR));
        assert!(is_staff_permissions(Permissions::MANAGE_MESSAGES));
        assert!(is_staff_permissions(Permissions::MANAGE_GUILD));
        assert!(!is_staff_permissions(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn permission_flags_map_staff_variants() {
        let flags = permission_flags(Permissions::ADMINISTRATOR);
        assert!(flags.author_is_admin);
        assert!(flags.author_is_staff);

        let flags = permission_flags(Permissions::MANAGE_MESSAGES);
        assert!(!flags.author_is_admin);
        assert!(flags.author_can_manage_messages);
        assert!(flags.author_is_staff);

        let flags = permission_flags(Permissions::MANAGE_GUILD);
        assert!(!flags.author_is_admin);
        assert!(flags.author_can_manage_guild);
        assert!(flags.author_is_staff);
    }

    #[test]
    fn image_attachment_detection_uses_filename_fallback_for_octet_stream() {
        assert!(is_image_attachment(
            Some("application/octet-stream"),
            "proof.PNG"
        ));
        assert!(is_image_attachment(Some("image/jpeg"), "upload.bin"));
        assert!(!is_image_attachment(Some("application/pdf"), "rules.pdf"));
    }

    #[test]
    fn image_attachment_detection_covers_discord_scam_exports() {
        for filename in [
            "IMG-20260502-WA3379.jpg",
            "1778988670733.jpg",
            "DSC_0113.jpg",
        ] {
            assert!(is_image_attachment(Some("image/jpeg"), filename));
            assert!(is_image_attachment(
                Some("application/octet-stream"),
                filename
            ));
        }
    }

    #[test]
    fn screening_completion_braucht_cache_after_pending() {
        let event = screening_completed_from_member_update(1, 2, true, None);

        assert!(
            event.is_none(),
            "fehlender Cache-After darf event.pending=false nicht als echten Übergang werten"
        );
    }

    #[test]
    fn screening_completion_erkennt_cache_transition() {
        let event = screening_completed_from_member_update(1, 2, true, Some(false));

        assert!(matches!(
            event,
            Some(MemberEvent::ScreeningCompleted {
                guild_id: 1,
                user_id: 2,
            })
        ));
    }

    #[test]
    fn completed_onboarding_flag_liefert_nur_echten_uebergang() {
        let event = completed_onboarding_from_member_update(
            1,
            2,
            serenity::all::GuildMemberFlags::empty(),
            Some(serenity::all::GuildMemberFlags::COMPLETED_ONBOARDING),
        );

        assert!(matches!(
            event,
            Some(MemberEvent::NativeOnboardingCompleted {
                guild_id: 1,
                user_id: 2,
            })
        ));
        assert!(
            completed_onboarding_from_member_update(
                1,
                2,
                serenity::all::GuildMemberFlags::COMPLETED_ONBOARDING,
                Some(serenity::all::GuildMemberFlags::COMPLETED_ONBOARDING),
            )
            .is_none(),
            "Rollenupdates bestehender Mitglieder duerfen kein Onboarding ausloesen"
        );
        assert!(completed_onboarding_from_member_update(
            1,
            2,
            serenity::all::GuildMemberFlags::empty(),
            None
        )
        .is_none());
    }

    #[test]
    fn reaction_self_ignore_erkennt_bot_id_und_application_id() {
        assert!(is_self_reaction_user(
            UserId::new(10),
            UserId::new(10),
            Some(20)
        ));
        assert!(is_self_reaction_user(
            UserId::new(20),
            UserId::new(10),
            Some(20)
        ));
        assert!(!is_self_reaction_user(
            UserId::new(30),
            UserId::new(10),
            Some(20)
        ));
    }
}
