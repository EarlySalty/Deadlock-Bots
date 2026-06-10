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

/// Normalisiertes Nachrichten-Ereignis (Bots bereits herausgefiltert).
#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub guild_id: Option<u64>,
    pub channel_id: u64,
    pub message_id: u64,
    pub author_id: u64,
    pub author_display_name: String,
    /// Aus dem Gateway-Cache berechnet; ohne Cache false.
    pub author_is_admin: bool,
    pub content: String,
    /// Anhänge gesamt / davon Bilder (für Spam-/Takeover-Detektion).
    pub attachment_count: u32,
    pub image_attachment_count: u32,
    /// Account-Erstellung (Unix, aus der Snowflake) und Guild-Join (Cache).
    pub author_created_at: i64,
    pub author_joined_at: Option<i64>,
}

/// Mitglieder-Ereignisse (join/remove) — Konsumenten: steam-bridge,
/// Onboarding (Phase 7), Aktivitäts-Analytik (Phase 5).
#[derive(Debug, Clone)]
pub enum MemberEvent {
    Join { guild_id: u64, user_id: u64 },
    Remove { guild_id: u64, user_id: u64 },
}

const CHANNEL_CAPACITY: usize = 1024;

pub struct Dispatcher {
    voice_tx: broadcast::Sender<VoiceEvent>,
    message_tx: broadcast::Sender<MessageEvent>,
    member_tx: broadcast::Sender<MemberEvent>,
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
        Self {
            voice_tx,
            message_tx,
            member_tx,
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
            content: "hallo".into(),
            attachment_count: 0,
            image_attachment_count: 0,
            author_created_at: 0,
            author_joined_at: None,
        });
    }
}
