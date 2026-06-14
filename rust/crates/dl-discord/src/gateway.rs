//! Gateway-Aufbau (serenity-Client). Start ist user-gated: Bis zum Cutover
//! hält der Python-Bot die Discord-Session; zwei aktive Handler würden
//! Events doppelt verarbeiten.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serenity::all::{
    Context, EventHandler, GatewayIntents, GuildId, GuildMemberUpdateEvent, InviteCreateEvent,
    InviteDeleteEvent, Member, Message, Ready, User, VoiceState,
};
use serenity::async_trait;

use crate::adapter::DiscordAdapter;
use crate::dispatcher::{Dispatcher, MemberEvent, MessageEvent, VoiceEvent};
use crate::interactions::InteractionRouter;
use crate::invite_tracker::InviteTracker;

struct Handler {
    adapter: Arc<DiscordAdapter>,
    dispatcher: Arc<Dispatcher>,
    router: Arc<InteractionRouter>,
    invite_tracker: Arc<InviteTracker>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        self.adapter.gateway_ready.store(true, Ordering::Relaxed);
        tracing::info!(user = %ready.user.name, guilds = ready.guilds.len(), "Gateway READY");
    }

    async fn cache_ready(&self, ctx: Context, guilds: Vec<GuildId>) {
        // Invite-Snapshots primen, damit der erste Join nach Start klassifiziert
        // werden kann (sonst „baseline_missing"). Joins treffen erst nach READY ein.
        for gid in &guilds {
            self.invite_tracker.prime(&ctx.http, gid.get()).await;
        }
        tracing::info!(guilds = guilds.len(), "Invite-Snapshots geprimt");
    }

    async fn interaction_create(&self, _ctx: Context, interaction: serenity::all::Interaction) {
        crate::dispatch::dispatch(&self.adapter, &self.router, &interaction).await;
    }

    async fn message(&self, ctx: Context, message: Message) {
        if message.author.bot {
            return;
        }
        // Admin-Flag aus dem Cache (für !steam_*-Admin-Kommandos u. ä.)
        let author_is_admin = message
            .guild_id
            .and_then(|guild_id| {
                let guild = ctx.cache.guild(guild_id)?;
                let member = guild.members.get(&message.author.id)?;
                Some(guild.member_permissions(member).administrator())
            })
            .unwrap_or(false);
        let image_attachment_urls: Vec<String> = message
            .attachments
            .iter()
            .filter(|a| {
                a.content_type
                    .as_deref()
                    .map(|c| c.starts_with("image/"))
                    .unwrap_or_else(|| {
                        let name = a.filename.to_lowercase();
                        [".png", ".jpg", ".jpeg", ".gif", ".webp"]
                            .iter()
                            .any(|ext| name.ends_with(ext))
                    })
            })
            .map(|a| a.url.clone())
            .collect();
        let image_attachment_count = image_attachment_urls.len() as u32;
        let author_joined_at = message.guild_id.and_then(|guild_id| {
            let guild = ctx.cache.guild(guild_id)?;
            let member = guild.members.get(&message.author.id)?;
            member.joined_at.map(|t| t.unix_timestamp())
        });
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
            content: message.content.clone(),
            is_reply: message.message_reference.is_some(),
            attachment_count: message.attachments.len() as u32,
            image_attachment_count,
            image_attachment_urls,
            author_created_at: message.author.id.created_at().unix_timestamp(),
            author_joined_at,
        });
    }

    async fn guild_member_addition(&self, ctx: Context, member: Member) {
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
        _member: Option<Member>,
    ) {
        self.dispatcher.publish_member(MemberEvent::Remove {
            guild_id: guild_id.get(),
            user_id: user.id.get(),
        });
    }

    async fn guild_member_update(
        &self,
        _ctx: Context,
        old: Option<Member>,
        new: Option<Member>,
        event: GuildMemberUpdateEvent,
    ) {
        // Neu hinzugekommene Rollen diffen (Onboarding-Verifikations-Abschluss).
        // Vorher-Rollen aus dem Cache (old); ohne Cache kein Diff möglich.
        let Some(old) = old else {
            return;
        };
        let after_roles: &[serenity::all::RoleId] = new
            .as_ref()
            .map(|m| m.roles.as_slice())
            .unwrap_or(&event.roles);
        let gained: Vec<u64> = after_roles
            .iter()
            .filter(|r| !old.roles.contains(r))
            .map(|r| r.get())
            .collect();
        if gained.is_empty() {
            return;
        }
        self.dispatcher
            .publish_role(crate::dispatcher::RoleEvent::Gained {
                guild_id: event.guild_id.get(),
                user_id: event.user.id.get(),
                role_ids: gained,
            });
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
}

/// Baut den serenity-Client. Aufrufer entscheidet über den Start
/// (DL_BOT_GATEWAY=1) und besitzt die Laufzeit-Task.
pub async fn build_client(
    token: &str,
    adapter: Arc<DiscordAdapter>,
    dispatcher: Arc<Dispatcher>,
    router: Arc<InteractionRouter>,
) -> serenity::Result<serenity::Client> {
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_INVITES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    serenity::Client::builder(token, intents)
        .event_handler(Handler {
            adapter,
            dispatcher,
            router,
            invite_tracker: Arc::new(InviteTracker::new()),
        })
        .await
}
