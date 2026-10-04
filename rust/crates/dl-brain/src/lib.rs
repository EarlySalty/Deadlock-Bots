use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainConfig {
    pub max_question_len: usize,
    pub cooldown_secs: u64,
}

#[derive(Default)]
pub struct BrainCooldowns {
    legacy: Mutex<HashMap<u64, Instant>>,
    discord: Mutex<DiscordRateState>,
}

impl BrainCooldowns {
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, HashMap<u64, Instant>> {
        self.legacy.lock().await
    }
}

#[derive(Default)]
struct DiscordRateState {
    users: HashMap<u64, Instant>,
    channels: HashMap<u64, VecDeque<Instant>>,
    day: u64,
    daily_count: usize,
}

impl DiscordRateState {
    fn reserve(&mut self, user_id: u64, channel_id: u64, now: Instant, utc: Duration) -> bool {
        self.users
            .retain(|_, last| now.saturating_duration_since(*last) < Duration::from_secs(60));
        self.channels.retain(|_, times| {
            while times.front().is_some_and(|last| {
                now.saturating_duration_since(*last) >= Duration::from_secs(3600)
            }) {
                times.pop_front();
            }
            !times.is_empty()
        });
        let day = utc.as_secs() / 86400;
        if self.day != day {
            self.day = day;
            self.daily_count = 0;
        }
        if user_id == 0
            || channel_id == 0
            || self.users.contains_key(&user_id)
            || self
                .channels
                .get(&channel_id)
                .is_some_and(|times| times.len() >= 20)
            || self.daily_count >= 500
        {
            return false;
        }
        self.users.insert(user_id, now);
        self.channels.entry(channel_id).or_default().push_back(now);
        self.daily_count += 1;
        true
    }
}

#[derive(Debug, Clone, Error)]
pub enum BrainError {
    #[error("brain backend failed: {0}")]
    Backend(String),
}

#[async_trait::async_trait]
pub trait AiAnswerer: Send + Sync {
    /// Führt Retrieval und genau eine gemeinsame Generierung aus.
    async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError>;

    async fn answer_for_discord(
        &self,
        _question: &str,
        _user_id: u64,
    ) -> Result<BrainOutcome, BrainError> {
        Err(BrainError::Backend(
            "Discord-Kontext für diesen Consumer nicht verfügbar".into(),
        ))
    }
}

pub async fn handle_discord_query(
    question: &str,
    user_id: u64,
    channel_id: u64,
    max_question_len: usize,
    cooldowns: &BrainCooldowns,
    answerer: &dyn AiAnswerer,
) -> Option<BrainOutcome> {
    let Ok(utc) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return None;
    };
    if !cooldowns
        .discord
        .lock()
        .await
        .reserve(user_id, channel_id, Instant::now(), utc)
    {
        return None;
    }
    let question = question.trim();
    if question.is_empty() {
        return Some(BrainOutcome::Usage);
    }
    let len = question.chars().count();
    if len > max_question_len {
        return Some(BrainOutcome::TooLong { len });
    }
    Some(match answerer.answer_for_discord(question, user_id).await {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::warn!("Discord-Brain-Anfrage fehlgeschlagen");
            BrainOutcome::BackendError
        }
    })
}

pub async fn handle_brain_query(
    question: &str,
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

    let outcome = match answerer.answer(question).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(%error, "Gemeinsame Brain-Antwort fehlgeschlagen");
            return BrainOutcome::BackendError;
        }
    };
    if !matches!(outcome, BrainOutcome::BackendError) {
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

    #[test]
    fn discord_limits_reservieren_nutzer_vor_dem_aufruf_auch_kanaluebergreifend() {
        let mut state = DiscordRateState::default();
        let now = Instant::now();
        let utc = Duration::from_secs(86400);
        assert!(state.reserve(1, 1, now, utc));
        assert!(!state.reserve(1, 2, now, utc));
        assert!(state.reserve(1, 2, now + Duration::from_secs(60), utc));
        assert_eq!(state.daily_count, 2);
    }

    #[test]
    fn discord_limits_zaehlen_zwanzig_pro_kanal_und_stunde() {
        let mut state = DiscordRateState::default();
        let now = Instant::now();
        let utc = Duration::from_secs(86400);
        for user in 1..=20 {
            assert!(state.reserve(user, 1, now, utc));
        }
        assert!(!state.reserve(21, 1, now, utc));
        assert!(state.reserve(21, 2, now, utc));
        assert!(state.reserve(22, 1, now + Duration::from_secs(3600), utc));
    }

    #[test]
    fn discord_limits_zaehlen_fuenfhundert_pro_utc_tag() {
        let mut state = DiscordRateState::default();
        let now = Instant::now();
        let utc = Duration::from_secs(86400);
        for user in 1..=500 {
            assert!(state.reserve(user, user, now, utc));
        }
        assert!(!state.reserve(501, 501, now, utc));
        assert!(state.reserve(501, 501, now, utc + Duration::from_secs(86400)));
    }

    #[tokio::test]
    async fn discord_verwendet_keinen_alten_antwortweg_als_ersatz() {
        let answerer = CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        let cooldowns = BrainCooldowns::default();
        assert_eq!(
            handle_discord_query("Frage", 3, 4, 300, &cooldowns, &answerer).await,
            Some(BrainOutcome::BackendError)
        );
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
        assert!(
            handle_discord_query("Frage", 3, 4, 300, &cooldowns, &answerer)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn parallele_discord_frage_wird_vor_der_ersten_antwort_begrenzt() {
        struct WaitingAnswerer {
            started: tokio::sync::Notify,
            release: tokio::sync::Notify,
        }
        #[async_trait::async_trait]
        impl AiAnswerer for WaitingAnswerer {
            async fn answer(&self, _question: &str) -> Result<BrainOutcome, BrainError> {
                Err(BrainError::Backend("Altweg unzulässig".into()))
            }
            async fn answer_for_discord(
                &self,
                _question: &str,
                _user_id: u64,
            ) -> Result<BrainOutcome, BrainError> {
                self.started.notify_one();
                self.release.notified().await;
                Ok(BrainOutcome::Answer("Antwort".into()))
            }
        }
        let answerer = WaitingAnswerer {
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        };
        let limits = BrainCooldowns::default();
        let (first, second) = tokio::join!(
            handle_discord_query("Erste Frage", 3, 4, 300, &limits, &answerer),
            async {
                answerer.started.notified().await;
                let result =
                    handle_discord_query("Zweite Frage", 3, 5, 300, &limits, &answerer).await;
                answerer.release.notify_one();
                result
            }
        );
        assert_eq!(first, Some(BrainOutcome::Answer("Antwort".into())));
        assert!(second.is_none());
    }

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
