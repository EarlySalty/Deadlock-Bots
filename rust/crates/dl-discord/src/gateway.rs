//! Gateway-Aufbau (serenity-Client). Start ist user-gated: Bis zum Cutover
//! hält der Python-Bot die Discord-Session; zwei aktive Handler würden
//! Events doppelt verarbeiten.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serenity::all::{Context, EventHandler, GatewayIntents, Message, Ready, VoiceState};
use serenity::async_trait;

use crate::adapter::DiscordAdapter;
use crate::dispatcher::{Dispatcher, MessageEvent, VoiceEvent};

struct Handler {
    adapter: Arc<DiscordAdapter>,
    dispatcher: Arc<Dispatcher>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        self.adapter.gateway_ready.store(true, Ordering::Relaxed);
        tracing::info!(user = %ready.user.name, guilds = ready.guilds.len(), "Gateway READY");
    }

    async fn message(&self, _ctx: Context, message: Message) {
        if message.author.bot {
            return;
        }
        self.dispatcher.publish_message(MessageEvent {
            guild_id: message.guild_id.map(|g| g.get()),
            channel_id: message.channel_id.get(),
            message_id: message.id.get(),
            author_id: message.author.id.get(),
            author_display_name: message
                .author_nick(&_ctx)
                .await
                .unwrap_or_else(|| message.author.name.to_string()),
            content: message.content.clone(),
        });
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
            _ => return, // Mute/Deaf-Änderungen ohne Kanalwechsel
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
        })
        .await
}
