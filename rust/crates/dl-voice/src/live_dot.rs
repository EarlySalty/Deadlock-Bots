use std::{sync::Arc, time::Duration};

use dl_discord::{DiscordAdapter, Dispatcher, GatewayEvent, RoleEvent, VoiceEvent};
use serenity::all::{
    ChannelId, ChannelType, Guild, GuildId, Permissions, RoleId, UserId, VoiceState,
};
use tokio::sync::broadcast::error::RecvError;

pub const LIVE_PREFIX: &str = "🟣 ";
pub const DEFAULT_LIVE_STREAMER_ROLE_ID: u64 = 1_560_993_753_643_098_153;
const MAX_CHANNEL_NAME_CHARS: usize = 100;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(180);

pub fn strip_live_prefix(name: &str) -> &str {
    let mut rest = name;
    while let Some(stripped) = rest.strip_prefix(LIVE_PREFIX) {
        rest = stripped;
    }
    rest
}

pub fn decorate_name(name: &str, live: bool) -> String {
    let base = strip_live_prefix(name);
    if !live {
        return base.to_string();
    }
    let room = MAX_CHANNEL_NAME_CHARS - LIVE_PREFIX.chars().count();
    let trimmed: String = base.chars().take(room).collect();
    format!("{LIVE_PREFIX}{trimmed}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveState {
    Ineligible,
    Unknown,
    Known(bool),
}

pub fn has_live_prefix(name: &str) -> bool {
    name.starts_with(LIVE_PREFIX)
}

pub fn decorate_for_state(name: &str, current: &str, state: LiveState) -> String {
    match state {
        LiveState::Ineligible => name.to_string(),
        LiveState::Unknown => decorate_name(name, has_live_prefix(current)),
        LiveState::Known(live) => decorate_name(name, live),
    }
}

fn fold_member_states(states: impl IntoIterator<Item = Option<bool>>) -> LiveState {
    let mut unknown = false;
    for state in states {
        match state {
            Some(true) => return LiveState::Known(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown {
        LiveState::Unknown
    } else {
        LiveState::Known(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveDotConfig {
    pub guild_id: u64,
    pub role_id: u64,
}

impl LiveDotConfig {
    pub fn new(guild_id: u64, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let role_id = lookup("DL_LIVE_STREAMER_ROLE_ID")
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_LIVE_STREAMER_ROLE_ID);
        Self { guild_id, role_id }
    }
}

pub struct LiveDot {
    adapter: Arc<DiscordAdapter>,
    config: LiveDotConfig,
}

impl LiveDot {
    pub fn new(adapter: Arc<DiscordAdapter>, config: LiveDotConfig) -> Arc<Self> {
        Arc::new(Self { adapter, config })
    }

    pub fn decorate(&self, channel_id: u64, name: &str, current: &str) -> String {
        decorate_for_state(name, current, self.channel_live(channel_id))
    }

    fn channel_live(&self, channel_id: u64) -> LiveState {
        let Some(guild) = self
            .adapter
            .cache()
            .guild(GuildId::new(self.config.guild_id))
        else {
            return LiveState::Ineligible;
        };
        let cid = ChannelId::new(channel_id);
        let Some(channel) = guild.channels.get(&cid) else {
            return LiveState::Ineligible;
        };
        if channel.kind != ChannelType::Voice {
            return LiveState::Ineligible;
        }
        let bot_id = self.adapter.cache().current_user().id;
        let Some(bot) = guild.members.get(&bot_id) else {
            return LiveState::Unknown;
        };
        if !guild
            .user_permissions_in(channel, bot)
            .contains(Permissions::MANAGE_CHANNELS)
        {
            return LiveState::Ineligible;
        }
        let role = RoleId::new(self.config.role_id);
        fold_member_states(
            guild
                .voice_states
                .values()
                .filter(|state| state.channel_id == Some(cid))
                .map(|state| member_has_role(&guild, state, role)),
        )
    }

    fn current_name(&self, channel_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(self.config.guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .map(|channel| channel.name.to_string())
    }

    fn voice_channel_ids(&self) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(self.config.guild_id))
            .map(|guild| {
                guild
                    .channels
                    .values()
                    .filter(|channel| channel.kind == ChannelType::Voice)
                    .map(|channel| channel.id.get())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn user_channel(&self, user_id: u64) -> Option<u64> {
        let guild = self
            .adapter
            .cache()
            .guild(GuildId::new(self.config.guild_id))?;
        guild
            .voice_states
            .get(&UserId::new(user_id))?
            .channel_id
            .map(|channel| channel.get())
    }

    pub async fn refresh(&self, channel_id: u64) {
        let LiveState::Known(live) = self.channel_live(channel_id) else {
            return;
        };
        let Some(current) = self.current_name(channel_id) else {
            return;
        };
        let target = decorate_name(&current, live);
        if target == current {
            return;
        }
        if let Err(err) =
            crate::rename_queue::enqueue_live_refresh(&self.adapter, channel_id, &target).await
        {
            tracing::warn!(%err, channel_id, "Live-Markierung: Umbenennung nicht eingereiht");
        }
    }

    pub async fn reconcile(&self) {
        for channel_id in self.voice_channel_ids() {
            self.refresh(channel_id).await;
        }
    }

    async fn on_voice(&self, event: VoiceEvent) {
        for channel_id in voice_event_channels(&event, self.config.guild_id) {
            self.refresh(channel_id).await;
        }
    }

    async fn on_role(&self, event: RoleEvent) {
        let (RoleEvent::Gained {
            guild_id,
            user_id,
            role_ids,
        }
        | RoleEvent::Removed {
            guild_id,
            user_id,
            role_ids,
        }) = event;
        if guild_id != self.config.guild_id || !role_ids.contains(&self.config.role_id) {
            return;
        }
        if let Some(channel_id) = self.user_channel(user_id) {
            self.refresh(channel_id).await;
        }
    }
}

fn member_has_role(guild: &Guild, state: &VoiceState, role: RoleId) -> Option<bool> {
    guild
        .members
        .get(&state.user_id)
        .or(state.member.as_ref())
        .map(|member| member.roles.contains(&role))
}

fn voice_event_channels(event: &VoiceEvent, guild: u64) -> Vec<u64> {
    match *event {
        VoiceEvent::Join {
            guild_id,
            channel_id,
            ..
        }
        | VoiceEvent::Leave {
            guild_id,
            channel_id,
            ..
        } if guild_id == guild => vec![channel_id],
        VoiceEvent::Move {
            guild_id,
            from_channel_id,
            to_channel_id,
            ..
        } if guild_id == guild => vec![from_channel_id, to_channel_id],
        _ => Vec::new(),
    }
}

pub fn spawn(live_dot: Arc<LiveDot>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut voice = dispatcher.subscribe_voice();
    let mut roles = dispatcher.subscribe_roles();
    let mut gateway = dispatcher.subscribe_gateway();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RECONCILE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                event = voice.recv() => match event {
                    Ok(event) => live_dot.on_voice(event).await,
                    Err(RecvError::Lagged(_)) => live_dot.reconcile().await,
                    Err(RecvError::Closed) => break,
                },
                event = roles.recv() => match event {
                    Ok(event) => live_dot.on_role(event).await,
                    Err(RecvError::Lagged(_)) => live_dot.reconcile().await,
                    Err(RecvError::Closed) => break,
                },
                event = gateway.recv() => match event {
                    Ok(GatewayEvent::CacheReady { .. }) | Err(RecvError::Lagged(_)) => {
                        live_dot.reconcile().await
                    }
                    Ok(GatewayEvent::Ready { .. }) => {}
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => live_dot.reconcile().await,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_prefix_when_live() {
        assert_eq!(decorate_name("Chill 1", true), "🟣 Chill 1");
    }

    #[test]
    fn strips_prefix_when_not_live() {
        assert_eq!(decorate_name("🟣 Chill 1", false), "Chill 1");
        assert_eq!(decorate_name("Chill 1", false), "Chill 1");
    }

    #[test]
    fn never_doubles_prefix() {
        assert_eq!(decorate_name("🟣 Chill 1", true), "🟣 Chill 1");
        assert_eq!(decorate_name("🟣 🟣 Chill 1", true), "🟣 Chill 1");
        assert_eq!(strip_live_prefix("🟣 🟣 Chill 1"), "Chill 1");
    }

    #[test]
    fn decoration_is_idempotent() {
        let once = decorate_name("Lane 3", true);
        assert_eq!(decorate_name(&once, true), once);
        let off = decorate_name(&once, false);
        assert_eq!(decorate_name(&off, false), off);
    }

    #[test]
    fn unknown_state_keeps_current_prefix() {
        assert_eq!(
            decorate_for_state("Lane 3", "🟣 Lane 2", LiveState::Unknown),
            "🟣 Lane 3"
        );
        assert_eq!(
            decorate_for_state("Lane 3", "Lane 2", LiveState::Unknown),
            "Lane 3"
        );
        assert_eq!(
            decorate_for_state("Lane 3", "🟣 Lane 2", LiveState::Ineligible),
            "Lane 3"
        );
    }

    #[test]
    fn unknown_member_blocks_negative_answer() {
        assert_eq!(fold_member_states([Some(false), None]), LiveState::Unknown);
        assert_eq!(
            fold_member_states([None, Some(true)]),
            LiveState::Known(true)
        );
        assert_eq!(fold_member_states([]), LiveState::Known(false));
    }

    #[test]
    fn keeps_discord_name_limit() {
        let long = "x".repeat(100);
        assert_eq!(decorate_name(&long, true).chars().count(), 100);
    }
}
