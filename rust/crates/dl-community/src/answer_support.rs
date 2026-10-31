use tokio::sync::{Semaphore, SemaphorePermit};

const STATEFUL_SUPPORT_TURN_LIMIT: usize = 4;

struct StatefulSupportAdmission {
    semaphore: Semaphore,
}

impl StatefulSupportAdmission {
    const fn new() -> Self {
        Self {
            semaphore: Semaphore::const_new(STATEFUL_SUPPORT_TURN_LIMIT),
        }
    }

    async fn acquire(&self) -> SemaphorePermit<'_> {
        self.semaphore
            .acquire()
            .await
            .expect("stateful support admission is never closed")
    }
}

static STATEFUL_SUPPORT_ADMISSION: StatefulSupportAdmission = StatefulSupportAdmission::new();

pub(crate) async fn acquire_stateful_support_turn() -> SemaphorePermit<'static> {
    STATEFUL_SUPPORT_ADMISSION.acquire().await
}

#[cfg(test)]
pub(crate) mod test_logging {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(crate) struct LogCapture {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl LogCapture {
        pub(crate) fn text(&self) -> String {
            String::from_utf8_lossy(&self.bytes.lock().expect("log capture")).to_string()
        }
    }

    pub(crate) struct LogCaptureWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for LogCaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("log capture")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            LogCaptureWriter {
                bytes: self.bytes.clone(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct KnowledgeAnswer {
    pub answerable: bool,
    pub answer: Option<String>,
    #[serde(default)]
    pub sources: Vec<KnowledgeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct KnowledgeSource {
    pub title: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KnowledgeLookup {
    Answer(KnowledgeAnswer),
    Unanswerable,
    Transport,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn stateful_support_admission_teilt_exakt_vier_db_pfade() {
        let admission = Arc::new(StatefulSupportAdmission::new());
        let mut held = Vec::new();
        for _ in 0..STATEFUL_SUPPORT_TURN_LIMIT {
            held.push(
                admission
                    .semaphore
                    .try_acquire()
                    .expect("vier gemeinsame FAQ-/Concierge-Slots"),
            );
        }
        assert!(admission.semaphore.try_acquire().is_err());

        let attempted = Arc::new(tokio::sync::Notify::new());
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let db_stateful_path_claimed = Arc::new(AtomicBool::new(false));
        let attempted_wait = attempted.notified();
        let fifth = tokio::spawn({
            let admission = admission.clone();
            let attempted = attempted.clone();
            let entered = entered.clone();
            let release = release.clone();
            let db_stateful_path_claimed = db_stateful_path_claimed.clone();
            async move {
                attempted.notify_one();
                let _permit = admission.acquire().await;
                db_stateful_path_claimed.store(true, Ordering::SeqCst);
                entered.notify_one();
                release.notified().await;
            }
        });

        attempted_wait.await;
        assert!(
            !db_stateful_path_claimed.load(Ordering::SeqCst),
            "fuenfter Turn darf vor Permit keinen DB-Stateful-Pfad beanspruchen"
        );
        drop(held.pop());
        entered.notified().await;
        assert!(db_stateful_path_claimed.load(Ordering::SeqCst));

        release.notify_one();
        fifth.await.expect("fifth stateful turn");
    }
}

pub(crate) fn fits_discord_content(text: &str) -> bool {
    !text.trim().is_empty() && text.encode_utf16().count() <= 2000
}

#[cfg(test)]
pub(crate) mod test_brain {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use dl_brain::{
        AiAnswerer, BrainError, BrainOutcome, ConversationReply, DialogSpeaker,
        DiscordConversation, DiscordConversationRequest,
    };
    use tokio::sync::Notify;

    pub(crate) type ScriptedReply = Result<ConversationReply, BrainError>;

    pub(crate) fn evidence_reply(json: &str) -> ScriptedReply {
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        Ok(parsed["evidence"][0]["text"]
            .as_str()
            .map_or(ConversationReply::Silent, |text| {
                ConversationReply::Answered(text.to_owned())
            }))
    }

    #[derive(Default)]
    pub(crate) struct ScriptedBrain {
        replies: Mutex<VecDeque<ScriptedReply>>,
        default_reply: Option<ConversationReply>,
        gate: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
        pub(crate) calls: Arc<AtomicUsize>,
        pub(crate) bodies: Arc<Mutex<Vec<String>>>,
        pub(crate) requests: Mutex<Vec<DiscordConversationRequest>>,
    }

    impl ScriptedBrain {
        pub(crate) fn replying(replies: Vec<ScriptedReply>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                ..Self::default()
            })
        }

        pub(crate) fn answering(text: &str) -> Arc<Self> {
            Self::replying(vec![Ok(ConversationReply::Answered(text.to_owned()))])
        }

        pub(crate) fn always(text: &str) -> Arc<Self> {
            Arc::new(Self {
                default_reply: Some(ConversationReply::Answered(text.to_owned())),
                ..Self::default()
            })
        }

        pub(crate) fn failing() -> Arc<Self> {
            Self::replying(vec![Err(BrainError::Backend("scripted".into()))])
        }

        pub(crate) fn gate_first_call(&self) -> (Arc<Notify>, Arc<Notify>) {
            let gate = (Arc::new(Notify::new()), Arc::new(Notify::new()));
            *self.gate.lock().expect("gate") = Some(gate.clone());
            gate
        }

        pub(crate) fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    fn joined_question(task: &DiscordConversation) -> String {
        let (question, history) = match task {
            DiscordConversation::Concierge {
                question, history, ..
            }
            | DiscordConversation::Faq { question, history } => (question, history.as_slice()),
            DiscordConversation::PassiveHelp { message, .. }
            | DiscordConversation::VoiceHint { message }
            | DiscordConversation::InviteLounge { message } => (message, [].as_slice()),
        };
        history
            .iter()
            .filter(|entry| matches!(entry.speaker, DialogSpeaker::User))
            .map(|entry| entry.text.as_str())
            .chain([question.as_str()])
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[async_trait]
    impl AiAnswerer for ScriptedBrain {
        async fn answer(&self, _: &str) -> Result<BrainOutcome, BrainError> {
            Err(BrainError::Backend("untyped path".into()))
        }

        async fn converse(
            &self,
            request: &DiscordConversationRequest,
        ) -> Result<ConversationReply, BrainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let body = serde_json::json!({ "question": joined_question(&request.task) });
            self.bodies
                .lock()
                .expect("bodies")
                .extend([body.to_string()]);
            self.requests
                .lock()
                .expect("requests")
                .extend([request.clone()]);
            let gate = self.gate.lock().expect("gate").take();
            if let Some((started, release)) = gate {
                started.notify_one();
                release.notified().await;
            }
            self.replies
                .lock()
                .expect("replies")
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(self
                        .default_reply
                        .clone()
                        .unwrap_or(ConversationReply::Silent))
                })
        }
    }
}
