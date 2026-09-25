use std::collections::HashMap;
use std::time::Instant;

use thiserror::Error;
use tokio::sync::Mutex;

pub mod api_ingest;

pub const DISCORD_MESSAGE_LIMIT: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrainOutcome {
    Usage,
    TooLong { len: usize },
    Cooldown { remaining_secs: u64 },
    Answer(String),
    OutOfDomain,
    NoAnswer,
    BackendError,
    ImageError(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainConfig {
    pub max_question_len: usize,
    pub cooldown_secs: u64,
}

pub type BrainCooldowns = Mutex<HashMap<u64, Instant>>;

#[derive(Debug, Clone, Error)]
pub enum BrainError {
    #[error("brain backend failed: {0}")]
    Backend(String),
    #[error("{0}")]
    Image(&'static str),
}

#[async_trait::async_trait]
pub trait AiAnswerer: Send + Sync {
    async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError>;

    async fn answer_with_images(
        &self,
        question: &str,
        image_urls: &[String],
    ) -> Result<BrainOutcome, BrainError> {
        if !image_urls.is_empty() {
            return Err(BrainError::Image("Die Bildanalyse ist gerade nicht verfügbar. Deine Frage wurde nicht ohne das Bild beantwortet."));
        }
        self.answer(question).await
    }
}

pub async fn handle_brain_query(
    question: &str,
    user_id: u64,
    cfg: &BrainConfig,
    cooldowns: &BrainCooldowns,
    answerer: &dyn AiAnswerer,
) -> BrainOutcome {
    handle_brain_query_with_images(question, &[], user_id, cfg, cooldowns, answerer).await
}

pub async fn handle_brain_query_with_images(
    question: &str,
    image_urls: &[String],
    user_id: u64,
    cfg: &BrainConfig,
    cooldowns: &BrainCooldowns,
    answerer: &dyn AiAnswerer,
) -> BrainOutcome {
    let question = question.trim();
    if question.is_empty() {
        return BrainOutcome::Usage;
    }

    let len = question.chars().count();
    if len > cfg.max_question_len {
        return BrainOutcome::TooLong { len };
    }

    if cfg.cooldown_secs > 0 {
        let cooldown = std::time::Duration::from_secs(cfg.cooldown_secs);
        let map = cooldowns.lock().await;
        if let Some(last) = map.get(&user_id) {
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(*last);
            if elapsed < cooldown {
                return BrainOutcome::Cooldown {
                    remaining_secs: remaining_secs(cooldown - elapsed),
                };
            }
        }
    }

    let outcome = match answerer.answer_with_images(question, image_urls).await {
        Ok(outcome) => outcome,
        Err(BrainError::Image(message)) => return BrainOutcome::ImageError(message),
        Err(error) => {
            tracing::warn!(%error, "Gemeinsame Brain-Antwort fehlgeschlagen");
            return BrainOutcome::BackendError;
        }
    };
    if !matches!(
        outcome,
        BrainOutcome::BackendError | BrainOutcome::ImageError(_)
    ) {
        register_cooldown(user_id, cfg, cooldowns).await;
    }
    outcome
}

fn remaining_secs(duration: std::time::Duration) -> u64 {
    let secs = duration.as_secs();
    if duration.subsec_nanos() > 0 {
        secs.saturating_add(1)
    } else {
        secs
    }
}

async fn register_cooldown(user_id: u64, cfg: &BrainConfig, cooldowns: &BrainCooldowns) {
    if cfg.cooldown_secs == 0 {
        return;
    }
    let now = Instant::now();
    let cooldown = std::time::Duration::from_secs(cfg.cooldown_secs);
    let mut map = cooldowns.lock().await;
    map.retain(|_, last| now.saturating_duration_since(*last) < cooldown);
    map.insert(user_id, now);
}

pub fn chunk_message(message: &str, limit: usize) -> Vec<String> {
    let message = message.trim();
    if message.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in message.split_whitespace() {
        let word_len = word.chars().count();
        if word_len > limit {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.extend(split_long_word(word, limit));
            continue;
        }

        let current_len = current.chars().count();
        let next_len = if current.is_empty() {
            word_len
        } else {
            current_len + 1 + word_len
        };
        if next_len <= limit {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current.push_str(word);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_long_word(word: &str, limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in word.chars() {
        if current.chars().count() >= limit {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    struct CountingAnswerer {
        calls: AtomicUsize,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl AiAnswerer for CountingAnswerer {
        async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(BrainError::Backend("test".into()))
            } else {
                Ok(BrainOutcome::Answer(question.into()))
            }
        }
    }

    #[tokio::test]
    async fn unsupported_images_are_not_ignored_and_do_not_consume_cooldown() {
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        let answerer = CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        let images = vec!["https://cdn.discordapp.com/attachments/1/2/a.png".to_owned()];
        assert!(matches!(
            handle_brain_query_with_images("Frage", &images, 1, &config, &cooldowns, &answerer)
                .await,
            BrainOutcome::ImageError(_)
        ));
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
        assert!(cooldowns.lock().await.is_empty());
        assert_eq!(
            handle_brain_query("Frage", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::Answer("Frage".into())
        );
        assert!(matches!(
            handle_brain_query_with_images("Frage", &images, 1, &config, &cooldowns, &answerer)
                .await,
            BrainOutcome::Cooldown { .. }
        ));
    }

    #[tokio::test]
    async fn gemeinsame_antwort_erhaelt_frage_und_beachtet_grenzen_und_cooldown() {
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        let answerer = CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        assert_eq!(
            handle_brain_query("", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::Usage
        );
        assert_eq!(
            handle_brain_query(&"x".repeat(21), 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::TooLong { len: 21 }
        );
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            handle_brain_query("Frage", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::Answer("Frage".into())
        );
        assert!(matches!(
            handle_brain_query("Frage", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::Cooldown { .. }
        ));
        cooldowns
            .lock()
            .await
            .insert(1, Instant::now() - Duration::from_secs(21));
        assert!(matches!(
            handle_brain_query("Frage", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::Answer(_)
        ));
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bild_urls_werden_an_den_answerer_durchgereicht() {
        struct ImageAnswerer {
            images: AtomicUsize,
        }

        #[async_trait::async_trait]
        impl AiAnswerer for ImageAnswerer {
            async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError> {
                Ok(BrainOutcome::Answer(question.to_string()))
            }

            async fn answer_with_images(
                &self,
                question: &str,
                image_urls: &[String],
            ) -> Result<BrainOutcome, BrainError> {
                self.images.store(image_urls.len(), Ordering::SeqCst);
                self.answer(question).await
            }
        }

        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 0,
        };
        let cooldowns = BrainCooldowns::default();
        let answerer = ImageAnswerer {
            images: AtomicUsize::new(0),
        };
        let image_urls = vec!["https://cdn.discordapp.com/a.png".to_string()];

        assert_eq!(
            handle_brain_query_with_images(
                "Frage",
                &image_urls,
                1,
                &config,
                &cooldowns,
                &answerer,
            )
            .await,
            BrainOutcome::Answer("Frage".into())
        );
        assert_eq!(answerer.images.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn backendfehler_verbraucht_keinen_cooldown() {
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        let answerer = CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: true,
        };
        assert_eq!(
            handle_brain_query("Frage", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::BackendError
        );
        assert!(cooldowns.lock().await.is_empty());
    }

    #[test]
    fn chunk_message_splittet_ueber_2000_an_wortgrenzen() {
        let message = (0..450)
            .map(|idx| format!("wort{idx:03}"))
            .collect::<Vec<_>>()
            .join(" ");

        let chunks = chunk_message(&message, DISCORD_MESSAGE_LIMIT);

        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= DISCORD_MESSAGE_LIMIT));
        assert!(chunks
            .iter()
            .all(|chunk| !chunk.starts_with(' ') && !chunk.ends_with(' ')));
        assert_eq!(chunks.join(" "), message);
    }
}
