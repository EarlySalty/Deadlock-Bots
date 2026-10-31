use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use dl_brain::{
    AiAnswerer, ChatEntry, ConversationReply, DiscordConversation, DiscordConversationRequest,
};
use dl_discord::MessageEvent;
use tokio::sync::Mutex;

use crate::voice_change_hint::VoiceHintReplyPort;

const CONTEXT_AGE: Duration = Duration::from_secs(5 * 60);
const REPLY_GAP: Duration = Duration::from_secs(90);
const DEDUP_AGE: Duration = Duration::from_secs(24 * 60 * 60);
fn answer_body(text: &str) -> &str {
    ["\n\nQuelle: <", "\n\nGrafik: "]
        .iter()
        .filter_map(|marker| text.find(marker))
        .min()
        .map_or(text, |end| &text[..end])
}

#[derive(Default)]
struct State {
    recent: VecDeque<(u64, Instant, String)>,
    seen: VecDeque<(u64, Instant)>,
    answered: VecDeque<(String, Instant)>,
    attempts: VecDeque<Instant>,
    last_reply: Option<Instant>,
}

pub struct PassiveHelpResponder {
    channel_id: u64,
    command_prefix: String,
    brain: Arc<dyn AiAnswerer>,
    port: Arc<dyn VoiceHintReplyPort>,
    state: Mutex<State>,
    admission: tokio::sync::Semaphore,
}

impl PassiveHelpResponder {
    pub fn new(
        channel_id: u64,
        command_prefix: String,
        brain: Arc<dyn AiAnswerer>,
        port: Arc<dyn VoiceHintReplyPort>,
    ) -> Self {
        Self {
            channel_id,
            command_prefix,
            brain,
            port,
            state: Mutex::new(State::default()),
            admission: tokio::sync::Semaphore::new(1),
        }
    }

    pub async fn handle_message(&self, event: &MessageEvent) {
        let content = event.content.trim();
        let age = chrono::Utc::now().signed_duration_since(event.message_created_at);
        if age.num_seconds() > CONTEXT_AGE.as_secs() as i64 {
            tracing::debug!(
                message_id = event.message_id,
                "Automatische Hilfe überspringt eine veraltete Nachricht"
            );
            return;
        }
        if self.channel_id == 0
            || event.channel_id != self.channel_id
            || event.guild_id.is_none()
            || content.is_empty()
            || content.starts_with(['!', '/'])
            || (!self.command_prefix.is_empty() && content.starts_with(&self.command_prefix))
        {
            return;
        }
        let now = Instant::now();
        let recent_messages = {
            let mut state = self.state.lock().await;
            state
                .seen
                .retain(|(_, at)| now.duration_since(*at) < DEDUP_AGE);
            if state.seen.iter().any(|(id, _)| *id == event.message_id) {
                return;
            }
            state.seen.push_back((event.message_id, now));
            while state.seen.len() > 2048 {
                state.seen.pop_front();
            }
            state
                .recent
                .retain(|(_, at, _)| now.duration_since(*at) < CONTEXT_AGE);
            let recent_messages = state
                .recent
                .iter()
                .map(|(author, _, text)| ChatEntry {
                    author: author.to_string(),
                    text: text.clone(),
                })
                .collect::<Vec<_>>();
            state.recent.push_back((
                event.author_id,
                now.checked_sub(Duration::from_millis(age.num_milliseconds().max(0) as u64))
                    .unwrap_or(now),
                content.chars().take(500).collect(),
            ));
            while state.recent.len() > 6 {
                state.recent.pop_front();
            }
            if content.chars().count() > 1000
                || !passes_prefilter(content)
                || state
                    .last_reply
                    .is_some_and(|at| now.duration_since(at) < REPLY_GAP)
            {
                return;
            }
            recent_messages
        };
        let Ok(_permit) = self.admission.try_acquire() else {
            return;
        };
        {
            let mut state = self.state.lock().await;
            state
                .attempts
                .retain(|at| now.duration_since(*at) < Duration::from_secs(60 * 60));
            if state.attempts.len() >= 30 {
                tracing::debug!(
                    channel_id = event.channel_id,
                    "Automatische Hilfe: Stundenbudget erreicht"
                );
                return;
            }
            state.attempts.push_back(now);
        }
        let key = normalize(content);
        {
            let mut state = self.state.lock().await;
            state
                .answered
                .retain(|(_, at)| now.duration_since(*at) < Duration::from_secs(10 * 60));
            if state.answered.iter().any(|(previous, _)| previous == &key) {
                return;
            }
        }
        let request = DiscordConversationRequest {
            user_id: event.author_id,
            channel_id: event.channel_id,
            guild_id: event.guild_id,
            task: DiscordConversation::PassiveHelp {
                message: content.to_string(),
                recent_messages,
            },
        };
        let result =
            tokio::time::timeout(Duration::from_secs(55), self.brain.converse(&request)).await;
        let text = match result {
            Ok(Ok(ConversationReply::Answered(text)))
                if !text.trim().is_empty()
                    && answer_body(&text).chars().count() <= 650
                    && !has_forbidden_pause(&text) =>
            {
                text
            }
            Ok(Ok(reply)) => {
                let reason = match reply {
                    ConversationReply::Silent => "still",
                    ConversationReply::Answered(ref text)
                        if answer_body(text).chars().count() > 650 =>
                    {
                        "Antwort überschreitet das Kurzbudget"
                    }
                    ConversationReply::Answered(ref text) if has_forbidden_pause(text) => {
                        "Antwort enthält verbotene Gedankenstrichpausen"
                    }
                    ConversationReply::Answered(_) => "leere Antwort",
                };
                tracing::info!(
                    message_id = event.message_id,
                    reason,
                    "Automatische Hilfe bleibt still"
                );
                return;
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    message_id = event.message_id,
                    %error,
                    "Automatische Hilfe: Antwort fehlgeschlagen"
                );
                return;
            }
            Err(_) => {
                tracing::warn!(
                    message_id = event.message_id,
                    "Automatische Hilfe: Antwort hat das Zeitlimit erreicht"
                );
                return;
            }
        };
        if let Err(error) = self
            .port
            .reply_text(event.channel_id, event.message_id, &text)
            .await
        {
            tracing::warn!(
                message_id = event.message_id,
                %error,
                "Automatische Hilfe konnte nicht gesendet werden"
            );
            return;
        }
        let mut state = self.state.lock().await;
        state.last_reply = Some(Instant::now());
        state.answered.push_back((key, Instant::now()));
        tracing::info!(
            channel_id = event.channel_id,
            message_id = event.message_id,
            "Automatische Hilfe hat eine belegte Antwort gesendet"
        );
    }
}

fn has_forbidden_pause(text: &str) -> bool {
    text.contains('—')
        || [" – ", " - ", " -- "]
            .iter()
            .any(|pause| text.contains(pause))
}

fn normalize(content: &str) -> String {
    content
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

pub fn passes_prefilter(content: &str) -> bool {
    let lower = content.to_lowercase();
    let subject = [
        "sprachkan",
        "voice",
        "router",
        "discord",
        "bot",
        "client",
        "fähigkeit",
        "item",
        "rang-gate",
    ]
    .iter()
    .any(|word| lower.contains(word));
    let blocked = ["nicht", "kein", "fehl", "kaputt", "verschwund"]
        .iter()
        .any(|word| lower.contains(word));
    crate::voice_change_hint::passes_voice_change_prefilter(content)
        || (subject && blocked)
        || [
            "wie ",
            "wie?",
            "wo ",
            "was ",
            "welch",
            "wofür",
            "wozu",
            "woran",
            "womit",
            "wann ",
            "kann ich",
            "darf ich",
            "können wir",
            "warum",
            "wieso",
            "weshalb",
            "hilfe",
            "help",
            "geht nicht",
            "klappt nicht",
            "keine ahnung",
            "kein plan",
            "ka ",
            "ka,",
            "weiß",
            "weiss",
            "versteh",
            "funktioniert",
            "fehler",
            "problem",
            "kann man",
            "könnte",
            "geht das",
            "bekomme",
            "finde",
            "how ",
            "where ",
        ]
        .iter()
        .any(|signal| lower.contains(signal))
}

pub fn spawn(
    responder: Arc<PassiveHelpResponder>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => responder.handle_message(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(count, "Automatische Hilfe hat Nachrichten verpasst")
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer_support::test_brain::ScriptedBrain;
    use dl_brain::ConversationReply;
    use std::sync::Mutex as StdMutex;

    const GROUNDED: &str = "Tritt dem Deadlock Router bei und wähle deinen Spielmodus.";

    #[derive(Default)]
    struct Port(StdMutex<Vec<String>>);
    #[async_trait::async_trait]
    impl VoiceHintReplyPort for Port {
        async fn reply_text(&self, _: u64, _: u64, content: &str) -> Result<u64, String> {
            self.0
                .lock()
                .expect("Fixture-Mutex")
                .extend([content.to_owned()]);
            Ok(123)
        }
    }

    fn responder(brain: Arc<ScriptedBrain>) -> (PassiveHelpResponder, Arc<Port>) {
        let port = Arc::new(Port::default());
        (
            PassiveHelpResponder::new(123, "!".into(), brain, port.clone()),
            port,
        )
    }

    fn event(message_id: u64, content: &str) -> MessageEvent {
        MessageEvent {
            guild_id: Some(1),
            channel_id: 123,
            message_id,
            author_id: 42,
            author_display_name: "Testperson".into(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.into(),
            message_created_at: chrono::Utc::now(),
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: vec![],
            attachments: vec![],
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    #[tokio::test]
    async fn posts_only_answered_brain_replies() {
        for (brain, sent) in [
            (ScriptedBrain::answering(GROUNDED), 1),
            (ScriptedBrain::replying(Vec::new()), 0),
            (ScriptedBrain::failing(), 0),
            (
                ScriptedBrain::replying(vec![Ok(ConversationReply::Answered(" ".into()))]),
                0,
            ),
        ] {
            let (responder, port) = responder(brain.clone());
            responder
                .handle_message(&event(1, "Wie erstelle ich einen Sprachkanal?"))
                .await;
            assert_eq!(brain.call_count(), 1);
            assert_eq!(port.0.lock().expect("Fixture-Mutex").len(), sent);
        }
    }

    #[tokio::test]
    async fn unpassender_antwortstil_bleibt_ohne_umschreibung_still() {
        for text in [
            "x".repeat(651),
            "Geh hinein \u{2013} dann klappt es.".into(),
            "Geh hinein \u{2014} dann klappt es.".into(),
            "Geh hinein - dann klappt es.".into(),
            "Geh hinein -- dann klappt es.".into(),
        ] {
            let (responder, port) = responder(ScriptedBrain::answering(&text));
            responder.handle_message(&event(1, "wie geht das?")).await;
            assert!(port.0.lock().expect("Fixture-Mutex").is_empty());
        }
        let (responder, port) =
            responder(ScriptedBrain::answering("Das dauert 3\u{2013}4 Minuten."));
        responder.handle_message(&event(1, "wie geht das?")).await;
        assert_eq!(port.0.lock().expect("Fixture-Mutex").len(), 1);
    }

    #[test]
    fn vorfilter_laesst_bedienhilfe_durch_und_spart_smalltalk() {
        assert!(passes_prefilter(
            "kannst du nen server aufmachen, ka wie man das hier macht XD"
        ));
        for text in [
            "Was bedeutet das Rang-Gate?",
            "Kann ich den Sprachkanal umbenennen?",
            "Wofür ist der Router?",
            "voice channels weg?",
            "sprachkanäle fehlen",
            "Ich kann dem Sprachkanal nicht beitreten",
            "Ich kann die Fähigkeit nicht aktivieren",
            "Kannst du den Server aufmachen? Kein Plan.",
        ] {
            assert!(passes_prefilter(text), "{text}");
        }
        for text in [
            "server call?",
            "suchst noch?",
            "joa",
            "qp? dann Ghosta mitspielen wenn er will",
            "mach mal server auf",
            "heute nicht",
            "ich spiele nicht mehr",
            "nicht ranked",
            "Ranked hab ich eh nicht frei",
        ] {
            assert!(!passes_prefilter(text), "{text}");
        }
    }

    #[tokio::test]
    async fn verlauf_geht_als_kontext_mit_und_antwort_kommt_einmal() {
        let brain = ScriptedBrain::always(GROUNDED);
        let (responder, port) = responder(brain.clone());
        responder.handle_message(&event(1, "server call?")).await;
        let help = event(
            2,
            "kannst du nen server aufmachen, ka wie man das hier macht XD",
        );
        responder.handle_message(&help).await;
        responder.handle_message(&help).await;
        responder
            .handle_message(&event(3, "wie funktioniert das?"))
            .await;
        assert_eq!(port.0.lock().expect("Fixture-Mutex").len(), 1);
        assert_eq!(brain.call_count(), 1);
        let requests = brain.requests.lock().expect("requests");
        let dl_brain::DiscordConversation::PassiveHelp {
            recent_messages, ..
        } = &requests[0].task
        else {
            panic!("Passive Hilfe erwartet");
        };
        assert!(recent_messages
            .iter()
            .any(|line| line.text == "server call?"));
    }

    #[tokio::test]
    async fn fremde_kanaele_commands_und_dms_bleiben_still() {
        let brain = ScriptedBrain::always(GROUNDED);
        let (responder, port) = responder(brain.clone());
        let mut other = event(1, "wie erstelle ich einen server?");
        other.channel_id = 456;
        responder.handle_message(&other).await;
        let mut dm = event(2, "wie erstelle ich einen server?");
        dm.guild_id = None;
        responder.handle_message(&dm).await;
        responder
            .handle_message(&event(3, "!brain wie erstelle ich einen server?"))
            .await;
        responder
            .handle_message(&event(4, "/brain wie erstelle ich einen server?"))
            .await;
        assert_eq!(brain.call_count(), 0);
        assert!(port.0.lock().expect("Fixture-Mutex").is_empty());
    }

    #[tokio::test]
    async fn gleiche_hilfe_nach_cooldown_wird_nicht_doppelt_gesendet() {
        let brain = ScriptedBrain::always(GROUNDED);
        let (responder, port) = responder(brain.clone());
        responder
            .handle_message(&event(1, "wie funktioniert das?"))
            .await;
        responder.state.lock().await.last_reply = None;
        responder
            .handle_message(&event(2, "Wie funktioniert das?"))
            .await;
        assert_eq!(port.0.lock().expect("Fixture-Mutex").len(), 1);
        assert_eq!(brain.call_count(), 1);
    }
}
