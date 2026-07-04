use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use dl_ai::{GenerateRequest, TextGenerator};
use serde_json::{json, Map, Value};

pub const VOICE_CHANGE_HINT_REPLY: &str = "Kein Bug — wir haben das Voice-System umgebaut. Die alten Sprachkanäle sind weg, jetzt gibt's **einen** Deadlock Router: rein, Modus wählen, eigene Lane. Wie's genau läuft: https://discord.com/channels/1289721245281292288/1371952264620806214/1522862733857525780";

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
pub trait VoiceHintClassifier: Send + Sync {
    async fn classify_voice_change_confusion(&self, content: &str) -> Result<bool, String>;
}

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

pub struct OpenAiVoiceHintClassifier {
    generator: Arc<dyn TextGenerator>,
}

impl OpenAiVoiceHintClassifier {
    pub fn new(generator: Arc<dyn TextGenerator>) -> Self {
        Self { generator }
    }
}

#[async_trait::async_trait]
impl VoiceHintClassifier for OpenAiVoiceHintClassifier {
    async fn classify_voice_change_confusion(&self, content: &str) -> Result<bool, String> {
        let raw = self
            .generator
            .generate_text(GenerateRequest {
                prompt: voice_change_prompt(content),
                system_prompt: None,
                model: None,
                max_output_tokens: Some(4),
                temperature: 0.0,
            })
            .await
            .ok_or_else(|| "OpenAI lieferte keine Klassifikation".to_string())?;
        Ok(is_yes_answer(&raw))
    }
}

pub struct VoiceChangeHintResponder {
    enabled: bool,
    classifier: Option<Arc<dyn VoiceHintClassifier>>,
    port: Arc<dyn VoiceHintReplyPort>,
    sent_by_user: Mutex<HashMap<u64, Instant>>,
}

impl VoiceChangeHintResponder {
    pub fn new(
        enabled: bool,
        classifier: Option<Arc<dyn VoiceHintClassifier>>,
        port: Arc<dyn VoiceHintReplyPort>,
    ) -> Self {
        if !enabled {
            tracing::info!("Voice-Change-Hint deaktiviert (DL_VOICE_HINT_ENABLED)");
        } else if classifier.is_none() {
            tracing::info!("Voice-Change-Hint inaktiv: kein OpenAI-Client konfiguriert");
        }

        Self {
            enabled,
            classifier,
            port,
            sent_by_user: Mutex::new(HashMap::new()),
        }
    }

    pub async fn handle_message(&self, event: &dl_discord::MessageEvent) {
        if !self.enabled
            || event.guild_id.is_none()
            || is_ignored_channel(event.channel_id)
            || !passes_voice_change_prefilter(&event.content)
        {
            return;
        }
        if self.user_in_cooldown(event.author_id, Instant::now()) {
            return;
        }
        let Some(classifier) = &self.classifier else {
            return;
        };

        match classifier
            .classify_voice_change_confusion(&event.content)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(err) => {
                tracing::debug!(
                    %err,
                    channel_id = event.channel_id,
                    message_id = event.message_id,
                    "Voice-Change-Hint-Klassifikation fehlgeschlagen"
                );
                return;
            }
        }

        let reserved_at = Instant::now();
        if !self.reserve_user_reply(event.author_id, reserved_at) {
            return;
        }

        if let Err(err) = self
            .port
            .reply_text(event.channel_id, event.message_id, VOICE_CHANGE_HINT_REPLY)
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

pub fn is_yes_answer(raw: &str) -> bool {
    let normalized: String = dl_ai::strip_think(raw)
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect();
    normalized.trim().to_uppercase().starts_with("JA")
}

fn voice_change_prompt(content: &str) -> String {
    format!(
        "Kontext: Ein Discord-Server hat sein Voice-System umgebaut — die alten einzelnen Sprachkanäle wurden entfernt und durch EINEN 'Deadlock Router' ersetzt, über den man sich per Klick eine eigene Lane erstellt.\n\nAufgabe: Drückt die folgende Chat-Nachricht aus, dass die Person die alten/fehlenden Sprachkanäle sucht, sich über verschwundene Voice-Kanäle wundert oder beschwert, oder nicht weiß, wie/wo sie jetzt in Voice kommt? Antworte AUSSCHLIESSLICH mit einem einzigen Wort: JA oder NEIN.\n\nNachricht: \"{content}\""
    )
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StaticClassifier {
        answer: bool,
        calls: AtomicUsize,
    }

    impl StaticClassifier {
        fn new(answer: bool) -> Self {
            Self {
                answer,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl VoiceHintClassifier for StaticClassifier {
        async fn classify_voice_change_confusion(&self, _content: &str) -> Result<bool, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.answer)
        }
    }

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

    #[test]
    fn ja_nein_parsing_ist_strikt_auf_ja_prefix() {
        assert!(is_yes_answer("JA"));
        assert!(is_yes_answer("Ja, klar"));
        assert!(is_yes_answer("**JA**"));
        assert!(is_yes_answer("`JA`"));
        assert!(is_yes_answer("„JA\""));
        assert!(!is_yes_answer("NEIN"));
        assert!(!is_yes_answer("**NEIN**"));
        assert!(!is_yes_answer("vielleicht"));
    }

    #[tokio::test]
    async fn dedup_blockiert_zweiten_treffer_desselben_users() {
        let classifier = Arc::new(StaticClassifier::new(true));
        let port = Arc::new(RecordingPort::default());
        let responder = VoiceChangeHintResponder::new(
            true,
            Some(classifier.clone() as Arc<dyn VoiceHintClassifier>),
            port.clone() as Arc<dyn VoiceHintReplyPort>,
        );

        responder
            .handle_message(&event(10, 42, "wo sind die voice channels hin?"))
            .await;
        responder
            .handle_message(&event(11, 42, "warum sind die sprachkanäle weg?"))
            .await;

        assert_eq!(port.count(), 1);
        assert_eq!(classifier.calls(), 1);
    }

    #[tokio::test]
    async fn sendefehler_entfernt_user_wieder_aus_dedup() {
        let classifier = Arc::new(StaticClassifier::new(true));
        let port = Arc::new(RecordingPort {
            sent: Mutex::new(Vec::new()),
            fail_replies: true,
        });
        let responder = VoiceChangeHintResponder::new(
            true,
            Some(classifier.clone() as Arc<dyn VoiceHintClassifier>),
            port.clone() as Arc<dyn VoiceHintReplyPort>,
        );

        responder
            .handle_message(&event(10, 42, "wo sind die voice channels hin?"))
            .await;
        responder
            .handle_message(&event(11, 42, "warum sind die sprachkanäle weg?"))
            .await;

        assert_eq!(port.count(), 0);
        assert_eq!(classifier.calls(), 2);
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
            message_created_at: 0,
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
