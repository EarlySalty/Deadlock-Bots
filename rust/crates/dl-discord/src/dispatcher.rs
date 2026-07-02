//! Der zentrale Event-Dispatcher.
//!
//! Kernstück des Querschnitts-Redesigns: Im Python-Original hören 5 Cogs
//! unkoordiniert auf `voice_state_update` und 6 auf `message`. Hier
//! normalisiert GENAU EINE Stelle die Gateway-Events, Domänen subscriben
//! über tokio-broadcast-Kanäle. Lahme Subscriber verlieren alte Events
//! (Lagged) statt den Bot zu blockieren.

use tokio::sync::broadcast;

/// Normalisiertes Voice-Ereignis (aus `voice_state_update` abgeleitet).
#[derive(Debug, Clone)]
pub enum VoiceEvent {
    Join {
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
    },
    Leave {
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
    },
    Move {
        guild_id: u64,
        user_id: u64,
        from_channel_id: u64,
        to_channel_id: u64,
    },
    /// Zustandsänderung im selben Kanal (Mute/Deaf) — der Tracker braucht
    /// das für die Grace-Period-Logik.
    Update {
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        was_muted: bool,
        is_muted: bool,
    },
}

#[derive(Debug, Clone)]
pub enum ChannelEvent {
    VoiceCategoryChanged {
        guild_id: u64,
        channel_id: u64,
        before_category_id: Option<u64>,
        after_category_id: Option<u64>,
    },
    VoiceChannelUpdated {
        guild_id: u64,
        channel_id: u64,
    },
}

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    Ready { guild_count: usize },
    CacheReady { guild_ids: Vec<u64> },
}

/// Normalisierte Interaction-Metadaten. Optionen, Modal-Felder und Message-
/// Inhalte werden bewusst nicht gespeichert; `route` ist Command-Name oder
/// custom_id fuer stille Klick-/Interaktionsmetriken.
#[derive(Debug, Clone)]
pub struct InteractionEvent {
    pub guild_id: Option<u64>,
    pub channel_id: Option<u64>,
    pub message_id: Option<u64>,
    pub interaction_id: u64,
    pub user_id: u64,
    pub interaction_kind: &'static str,
    pub route: Option<String>,
    pub occurred_at: i64,
}

/// Normalisiertes Nachrichten-Ereignis (Bots bereits herausgefiltert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageAttachment {
    pub url: String,
    pub content_type: String,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub guild_id: Option<u64>,
    pub channel_id: u64,
    pub message_id: u64,
    pub author_id: u64,
    pub author_display_name: String,
    /// Aus dem Gateway-Cache berechnet; ohne Cache false.
    pub author_is_admin: bool,
    /// AI-Moderator-Staff-Skip wie Python: manage_messages.
    pub author_can_manage_messages: bool,
    /// Serverstats-Paritaet: administrator || manage_guild.
    pub author_can_manage_guild: bool,
    /// Staff-Schutz fuer Moderationspfade: administrator || manage_messages || manage_guild.
    pub author_is_staff: bool,
    /// `true`, wenn der Gateway-Cache den Member enthielt und Staff-Rechte
    /// berechnet werden konnten. SecurityGuard nutzt `false` fail-closed.
    pub author_staff_status_known: bool,
    pub content: String,
    /// Nachrichtenerstellung (Unix-Sekunden, aus Discord-Timestamp/Snowflake).
    pub message_created_at: i64,
    /// Ist die Nachricht eine Antwort (`message_reference` gesetzt)? Für den
    /// einmaligen Reply-Bonus der Text-Gamification.
    pub is_reply: bool,
    /// Ziel der Reply, falls vorhanden. Wird von der AI-Moderation fuer
    /// `is_reply_to`-Kontext aufgeloest.
    pub reply_message_id: Option<u64>,
    pub reply_channel_id: Option<u64>,
    /// Anhänge gesamt / davon Bilder (für Spam-/Takeover-Detektion).
    pub attachment_count: u32,
    pub image_attachment_count: u32,
    /// URLs der Bild-Anhänge (gleiche Filterung wie `image_attachment_count`)
    /// — für die Vision-Klassifikation der Moderation.
    pub image_attachment_urls: Vec<String>,
    /// Metadaten aller Anhänge für Moderations-Cases/Logs.
    pub attachments: Vec<MessageAttachment>,
    /// Account-Erstellung (Unix, aus der Snowflake) und Guild-Join (Cache).
    pub author_created_at: i64,
    pub author_joined_at: Option<i64>,
}

/// Mitglieder-Ereignisse — Konsumenten: steam-bridge, Onboarding (Phase 7),
/// Aktivitäts-Analytik (Phase 5), Leave-Survey.
#[derive(Debug, Clone)]
pub enum MemberEvent {
    Join {
        guild_id: u64,
        user_id: u64,
        display_name: String,
        /// Account-Erstellung (Unix-Sekunden, aus der Snowflake).
        account_created_at: i64,
        /// Mitgliederzahl der Gilde zum Join-Zeitpunkt (aus dem Cache).
        join_position: Option<i64>,
        is_bot: bool,
        /// Roh-Detektion der Beitrittsquelle (`join_source_*` + `invite_*` +
        /// `avatar_url`/`is_pending`) als JSON; der Writer verfeinert sie über
        /// `classify` (Twitch-/Website-Override) und persistiert sie.
        metadata: serde_json::Value,
    },
    Remove {
        guild_id: u64,
        user_id: u64,
        display_name: String,
        is_bot: bool,
    },
    Ban {
        guild_id: u64,
        user_id: u64,
        display_name: String,
        is_bot: bool,
    },
    Unban {
        guild_id: u64,
        user_id: u64,
        display_name: String,
        is_bot: bool,
    },
    /// Discord Member Screening wurde abgeschlossen (`pending: true -> false`).
    ScreeningCompleted { guild_id: u64, user_id: u64 },
}

/// Rollenänderung eines Mitglieds (aus `guild_member_update` diffiert) —
/// Konsumenten: Onboarding-Verifikations-Abschluss, Coaching-Roster-Sync.
#[derive(Debug, Clone)]
pub enum RoleEvent {
    Gained {
        guild_id: u64,
        user_id: u64,
        role_ids: Vec<u64>,
    },
    Removed {
        guild_id: u64,
        user_id: u64,
        role_ids: Vec<u64>,
    },
}

pub fn role_events_from_diff(
    guild_id: u64,
    user_id: u64,
    before_roles: &[u64],
    after_roles: &[u64],
) -> Vec<RoleEvent> {
    let gained: Vec<u64> = after_roles
        .iter()
        .copied()
        .filter(|role_id| !before_roles.contains(role_id))
        .collect();
    let removed: Vec<u64> = before_roles
        .iter()
        .copied()
        .filter(|role_id| !after_roles.contains(role_id))
        .collect();

    let mut events = Vec::with_capacity(2);
    if !gained.is_empty() {
        events.push(RoleEvent::Gained {
            guild_id,
            user_id,
            role_ids: gained,
        });
    }
    if !removed.is_empty() {
        events.push(RoleEvent::Removed {
            guild_id,
            user_id,
            role_ids: removed,
        });
    }
    events
}

pub fn member_screening_completed_event(
    guild_id: u64,
    user_id: u64,
    before_pending: bool,
    after_pending: bool,
) -> Option<MemberEvent> {
    if before_pending && !after_pending {
        Some(MemberEvent::ScreeningCompleted { guild_id, user_id })
    } else {
        None
    }
}

const CHANNEL_CAPACITY: usize = 1024;

pub struct Dispatcher {
    voice_tx: broadcast::Sender<VoiceEvent>,
    message_tx: broadcast::Sender<MessageEvent>,
    member_tx: broadcast::Sender<MemberEvent>,
    role_tx: broadcast::Sender<RoleEvent>,
    channel_tx: broadcast::Sender<ChannelEvent>,
    interaction_tx: broadcast::Sender<InteractionEvent>,
    gateway_tx: broadcast::Sender<GatewayEvent>,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    pub fn new() -> Self {
        let (voice_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (message_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (member_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (role_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (channel_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (interaction_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (gateway_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            voice_tx,
            message_tx,
            member_tx,
            role_tx,
            channel_tx,
            interaction_tx,
            gateway_tx,
        }
    }

    pub fn subscribe_voice(&self) -> broadcast::Receiver<VoiceEvent> {
        self.voice_tx.subscribe()
    }

    pub fn subscribe_messages(&self) -> broadcast::Receiver<MessageEvent> {
        self.message_tx.subscribe()
    }

    pub fn subscribe_members(&self) -> broadcast::Receiver<MemberEvent> {
        self.member_tx.subscribe()
    }

    pub fn subscribe_roles(&self) -> broadcast::Receiver<RoleEvent> {
        self.role_tx.subscribe()
    }

    pub fn subscribe_channels(&self) -> broadcast::Receiver<ChannelEvent> {
        self.channel_tx.subscribe()
    }

    pub fn subscribe_interactions(&self) -> broadcast::Receiver<InteractionEvent> {
        self.interaction_tx.subscribe()
    }

    pub fn subscribe_gateway(&self) -> broadcast::Receiver<GatewayEvent> {
        self.gateway_tx.subscribe()
    }

    pub fn publish_voice(&self, event: VoiceEvent) {
        // send schlägt nur fehl, wenn niemand subscribed ist — kein Fehler.
        let _ = self.voice_tx.send(event);
    }

    pub fn publish_message(&self, event: MessageEvent) {
        let _ = self.message_tx.send(event);
    }

    pub fn publish_member(&self, event: MemberEvent) {
        let _ = self.member_tx.send(event);
    }

    pub fn publish_channel(&self, event: ChannelEvent) {
        let _ = self.channel_tx.send(event);
    }

    pub fn publish_interaction(&self, event: InteractionEvent) {
        let _ = self.interaction_tx.send(event);
    }

    pub fn publish_gateway(&self, event: GatewayEvent) {
        let _ = self.gateway_tx.send(event);
    }

    pub fn publish_role(&self, event: RoleEvent) {
        let _ = self.role_tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn voice_events_erreichen_mehrere_subscriber() {
        let dispatcher = Dispatcher::new();
        let mut a = dispatcher.subscribe_voice();
        let mut b = dispatcher.subscribe_voice();
        dispatcher.publish_voice(VoiceEvent::Join {
            guild_id: 1,
            user_id: 2,
            channel_id: 3,
        });
        assert!(matches!(
            a.recv().await.expect("a"),
            VoiceEvent::Join { user_id: 2, .. }
        ));
        assert!(matches!(
            b.recv().await.expect("b"),
            VoiceEvent::Join { user_id: 2, .. }
        ));
    }

    #[tokio::test]
    async fn publish_ohne_subscriber_ist_ok() {
        let dispatcher = Dispatcher::new();
        dispatcher.publish_message(MessageEvent {
            guild_id: None,
            channel_id: 1,
            message_id: 2,
            author_id: 3,
            author_display_name: "x".into(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: "hallo".into(),
            message_created_at: 0,
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        });
    }

    #[test]
    fn rollen_diff_liefert_gain_und_remove_events() {
        let events = role_events_from_diff(1, 2, &[10, 20, 30], &[20, 30, 40]);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            RoleEvent::Gained {
                guild_id: 1,
                user_id: 2,
                ref role_ids,
            } if role_ids == &[40]
        ));
        assert!(matches!(
            events[1],
            RoleEvent::Removed {
                guild_id: 1,
                user_id: 2,
                ref role_ids,
            } if role_ids == &[10]
        ));
    }

    #[test]
    fn pending_transition_liefert_screening_completed_event() {
        let event = member_screening_completed_event(1, 2, true, false);
        assert!(matches!(
            event,
            Some(MemberEvent::ScreeningCompleted {
                guild_id: 1,
                user_id: 2,
            })
        ));
        assert!(member_screening_completed_event(1, 2, false, false).is_none());
        assert!(member_screening_completed_event(1, 2, true, true).is_none());
    }
}
