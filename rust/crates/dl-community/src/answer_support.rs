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
    #[cfg(test)]
    Timeout,
    Transport,
    #[cfg(test)]
    InvalidResponse,
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
