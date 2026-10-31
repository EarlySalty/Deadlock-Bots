use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use dl_brain::{AiAnswerer, ConversationReply, DiscordConversation, DiscordConversationRequest};
use serde_json::{json, Map, Value};

const ANNOUNCEMENTS_CHANNEL_ID: u64 = 1_371_952_264_620_806_214;
const LOG_CHANNEL_ID: u64 = 1_374_364_800_817_303_632;
const DEDUP_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

const VOICE_WORDS: &[&str] = &[
    "voice",
    "sprachkanal",
    "sprachkanäle",
    "sprachkanaele",
    "kanäle",
    "kanaele",
    "channels",
    "channel",
    "lane",
    "lanes",
    "vc",
];

const MISSING_OR_WHERE_WORDS: &[&str] = &[
    "weg",
    "fehlt",
    "fehlen",
    "verschwund",
    "gelöscht",
    "geloescht",
    "wo ist",
    "wo sind",
    "wo finde",
    "wo kann",
    "wieso",
    "warum",
    "kein voice",
    "keine kanäle",
    "nicht mehr",
    "wohin",
];

#[async_trait::async_trait]
pub trait VoiceHintReplyPort: Send + Sync {
    async fn reply_text(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<u64, String>;
}

#[async_trait::async_trait]
impl VoiceHintReplyPort for dl_discord::DiscordAdapter {
    async fn reply_text(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<u64, String> {
        let mut body = Map::new();
        body.insert("content".into(), Value::String(content.to_string()));
        body.insert(
            "message_reference".into(),
            json!({
                "channel_id": channel_id.to_string(),
                "message_id": message_id.to_string(),
                "fail_if_not_exists": false,
            }),
        );
        body.insert(
            "allowed_mentions".into(),
            json!({
                "parse": [],
                "replied_user": false,
            }),
        );
        self.send_raw_public(channel_id, &body).await
    }
}

pub struct VoiceChangeHintResponder {
    enabled: bool,
    brain: Arc<dyn AiAnswerer>,
    port: Arc<dyn VoiceHintReplyPort>,
    sent_by_user: Mutex<HashMap<u64, Instant>>,
    excluded_channel: Option<u64>,
}

impl VoiceChangeHintResponder {
    pub fn new(
        enabled: bool,
        brain: Arc<dyn AiAnswerer>,
        port: Arc<dyn VoiceHintReplyPort>,
    ) -> Self {
        if !enabled {
            tracing::info!("Voice-Change-Hint deaktiviert (DL_VOICE_HINT_ENABLED)");
        }

        Self {
            enabled,
            brain,
            port,
            sent_by_user: Mutex::new(HashMap::new()),
            excluded_channel: None,
        }
    }

    pub fn excluding_channel(mut self, channel_id: Option<u64>) -> Self {
        self.excluded_channel = channel_id;
        self
    }

    pub async fn handle_message(&self, event: &dl_discord::MessageEvent) {
        if !self.enabled
            || self.excluded_channel == Some(event.channel_id)
            || event.guild_id.is_none()
            || is_ignored_channel(event.channel_id)
            || !passes_voice_change_prefilter(&event.content)
        {
            return;
        }
        if self.user_in_cooldown(event.author_id, Instant::now()) {
            return;
        }
        let reserved_at = Instant::now();
        if !self.reserve_user_reply(event.author_id, reserved_at) {
            return;
        }
        let request = DiscordConversationRequest {
            user_id: event.author_id,
            channel_id: event.channel_id,
            guild_id: event.guild_id,
            task: DiscordConversation::VoiceHint {
                message: event.content.clone(),
            },
        };
        let text = match self.brain.converse(&request).await {
            Ok(ConversationReply::Answered(text))
                if crate::answer_support::fits_discord_content(&text) =>
            {
                text
            }
            Ok(_) => {
                self.release_user_reply(event.author_id);
                return;
            }
            Err(err) => {
                tracing::debug!(
                    %err,
                    channel_id = event.channel_id,
                    message_id = event.message_id,
                    "Voice-Change-Hint-Antwort fehlgeschlagen"
                );
                self.release_user_reply(event.author_id);
                return;
            }
        };
        if let Err(err) = self
            .port
            .reply_text(event.channel_id, event.message_id, &text)
            .await
        {
            tracing::warn!(
                %err,
                channel_id = event.channel_id,
                message_id = event.message_id,
                "Voice-Change-Hint-Reply konnte nicht gesendet werden"
            );
            self.release_user_reply(event.author_id);
        }
    }

    fn user_in_cooldown(&self, user_id: u64, now: Instant) -> bool {
        let Ok(mut sent_by_user) = self.sent_by_user.lock() else {
            tracing::warn!("Voice-Change-Hint-Dedup-Mutex ist vergiftet");
            return true;
        };
        prune_expired(&mut sent_by_user, now);
        sent_by_user
            .get(&user_id)
            .is_some_and(|last_sent_at| !dedup_expired(now, *last_sent_at))
    }

    fn reserve_user_reply(&self, user_id: u64, now: Instant) -> bool {
        let Ok(mut sent_by_user) = self.sent_by_user.lock() else {
            tracing::warn!("Voice-Change-Hint-Dedup-Mutex ist vergiftet");
            return false;
        };
        prune_expired(&mut sent_by_user, now);
        if sent_by_user
            .get(&user_id)
            .is_some_and(|last_sent_at| !dedup_expired(now, *last_sent_at))
        {
            return false;
        }
        sent_by_user.insert(user_id, now);
        true
    }

    fn release_user_reply(&self, user_id: u64) {
        let Ok(mut sent_by_user) = self.sent_by_user.lock() else {
            tracing::warn!("Voice-Change-Hint-Dedup-Mutex ist vergiftet");
            return;
        };
        sent_by_user.remove(&user_id);
    }
}

pub fn spawn(
    responder: Arc<VoiceChangeHintResponder>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => responder.handle_message(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn enabled_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> bool {
    lookup("DL_VOICE_HINT_ENABLED")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "0" | "false")
        })
        .unwrap_or(true)
}

pub fn passes_voice_change_prefilter(content: &str) -> bool {
    let lower = content.to_lowercase();
    VOICE_WORDS.iter().any(|word| lower.contains(word))
        && MISSING_OR_WHERE_WORDS
            .iter()
            .any(|word| lower.contains(word))
}

fn is_ignored_channel(channel_id: u64) -> bool {
    matches!(channel_id, ANNOUNCEMENTS_CHANNEL_ID | LOG_CHANNEL_ID)
}

fn prune_expired(sent_by_user: &mut HashMap<u64, Instant>, now: Instant) {
    sent_by_user.retain(|_, last_sent_at| !dedup_expired(now, *last_sent_at));
}

fn dedup_expired(now: Instant, last_sent_at: Instant) -> bool {
    now.checked_duration_since(last_sent_at)
        .is_some_and(|elapsed| elapsed >= DEDUP_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer_support::test_brain::ScriptedBrain;

    #[derive(Default)]
    struct RecordingPort {
        sent: Mutex<Vec<(u64, u64, String)>>,
        fail_replies: bool,
    }

    impl RecordingPort {
        fn count(&self) -> usize {
            self.sent.lock().map(|sent| sent.len()).unwrap_or(0)
        }
    }

    #[async_trait::async_trait]
    impl VoiceHintReplyPort for RecordingPort {
        async fn reply_text(
            &self,
            channel_id: u64,
            message_id: u64,
            content: &str,
        ) -> Result<u64, String> {
            if self.fail_replies {
                return Err("Reply fehlgeschlagen".to_string());
            }
            let mut sent = self
                .sent
                .lock()
                .map_err(|_| "RecordingPort-Mutex vergiftet".to_string())?;
            sent.push((channel_id, message_id, content.to_string()));
            Ok(sent.len() as u64)
        }
    }

    #[test]
    fn vorfilter_erkennt_voice_und_wo_signal() {
        assert!(passes_voice_change_prefilter(
            "wo sind die voice channels hin?"
        ));
        assert!(!passes_voice_change_prefilter(
            "ich liebe diesen channel hier"
        ));
        assert!(!passes_voice_change_prefilter("hallo zusammen"));
    }

    #[tokio::test]
    async fn dedup_blockiert_zweiten_treffer_desselben_users() {
        let brain = ScriptedBrain::always("Brain-Hinweis");
        let port = Arc::new(RecordingPort::default());
        let responder = VoiceChangeHintResponder::new(true, brain.clone(), port.clone());

        responder
            .handle_message(&event(10, 42, "wo sind die voice channels hin?"))
            .await;
        responder
            .handle_message(&event(11, 42, "warum sind die sprachkanäle weg?"))
            .await;

        assert_eq!(port.count(), 1);
        assert_eq!(brain.call_count(), 1);
    }

    #[tokio::test]
    async fn sendefehler_entfernt_user_wieder_aus_dedup() {
        let brain = ScriptedBrain::always("Brain-Hinweis");
        let port = Arc::new(RecordingPort {
            sent: Mutex::new(Vec::new()),
            fail_replies: true,
        });
        let responder = VoiceChangeHintResponder::new(true, brain.clone(), port.clone());

        responder
            .handle_message(&event(10, 42, "wo sind die voice channels hin?"))
            .await;
        responder
            .handle_message(&event(11, 42, "warum sind die sprachkanäle weg?"))
            .await;

        assert_eq!(port.count(), 0);
        assert_eq!(brain.call_count(), 2);
    }

    #[tokio::test]
    async fn stilles_brain_sendet_nichts_und_haelt_keine_sperre() {
        for brain in [
            ScriptedBrain::replying(Vec::new()),
            ScriptedBrain::failing(),
        ] {
            let port = Arc::new(RecordingPort::default());
            let responder = VoiceChangeHintResponder::new(true, brain.clone(), port.clone());
            responder
                .handle_message(&event(10, 42, "wo sind die voice channels hin?"))
                .await;
            responder
                .handle_message(&event(11, 42, "warum sind die sprachkanäle weg?"))
                .await;
            assert_eq!(port.count(), 0);
            assert_eq!(brain.call_count(), 2);
        }
    }

    fn event(message_id: u64, author_id: u64, content: &str) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id: Some(1_289_721_245_281_292_288),
            channel_id: 123,
            message_id,
            author_id,
            author_display_name: "User".to_string(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.to_string(),
            message_created_at: chrono::DateTime::from_timestamp(0, 0).expect("fixture timestamp"),
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: Some(0),
        }
    }
}
