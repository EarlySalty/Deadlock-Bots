use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Europe::Berlin;

use thiserror::Error;
use tokio::sync::Mutex;

pub mod api_ingest;
pub mod brain_api;

pub const DISCORD_MESSAGE_LIMIT: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrainOutcome {
    Usage,
    TooLong { len: usize },
    Cooldown { remaining_secs: u64 },
    DailyLimit,
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

pub const DEFAULT_DISCORD_DAILY_USER_LIMIT: usize = 50;

pub struct BrainCooldowns {
    legacy: Mutex<HashMap<u64, Instant>>,
    discord: Mutex<DiscordRateState>,
    daily_user_limit: usize,
}

impl Default for BrainCooldowns {
    fn default() -> Self {
        Self::new(DEFAULT_DISCORD_DAILY_USER_LIMIT)
    }
}

impl BrainCooldowns {
    pub fn new(daily_user_limit: usize) -> Self {
        Self {
            legacy: Mutex::default(),
            discord: Mutex::default(),
            daily_user_limit,
        }
    }

    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, HashMap<u64, Instant>> {
        self.legacy.lock().await
    }

    pub async fn reserve_discord(&self, user_id: u64, channel_id: u64) -> DiscordReservation {
        self.discord
            .lock()
            .await
            .reserve(user_id, channel_id, Utc::now(), self.daily_user_limit)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordReservation {
    Accepted,
    DailyLimit,
    Suppressed,
}

#[derive(Default)]
struct DiscordUserDay {
    count: usize,
    notified: bool,
}

#[derive(Default)]
struct DiscordRateState {
    users: HashMap<u64, DiscordUserDay>,
    day: Option<NaiveDate>,
}

impl DiscordRateState {
    fn reserve(
        &mut self,
        user_id: u64,
        channel_id: u64,
        utc: DateTime<Utc>,
        daily_user_limit: usize,
    ) -> DiscordReservation {
        if user_id == 0 || channel_id == 0 {
            return DiscordReservation::Suppressed;
        }
        let day = utc.with_timezone(&Berlin).date_naive();
        if self.day != Some(day) {
            self.day = Some(day);
            self.users.clear();
        }
        let user = self.users.entry(user_id).or_default();
        if user.count >= daily_user_limit {
            if user.notified {
                return DiscordReservation::Suppressed;
            }
            user.notified = true;
            return DiscordReservation::DailyLimit;
        }
        user.count += 1;
        DiscordReservation::Accepted
    }
}

#[derive(Debug, Clone, Error)]
pub enum BrainError {
    #[error("brain backend failed: {0}")]
    Backend(String),
}

#[async_trait::async_trait]
pub trait AiAnswerer: Send + Sync {
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

    async fn answer_for_discord_with_read_access(
        &self,
        question: &str,
        user_id: u64,
        allow_discord_reads: bool,
    ) -> Result<BrainOutcome, BrainError> {
        if allow_discord_reads {
            self.answer_for_discord(question, user_id).await
        } else {
            Err(BrainError::Backend(
                "Discord-Lesesperre für diesen Consumer nicht verfügbar".into(),
            ))
        }
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
    handle_discord_query_with_read_access(
        question,
        user_id,
        channel_id,
        max_question_len,
        cooldowns,
        answerer,
        true,
    )
    .await
}

pub async fn handle_discord_query_with_read_access(
    question: &str,
    user_id: u64,
    channel_id: u64,
    max_question_len: usize,
    cooldowns: &BrainCooldowns,
    answerer: &dyn AiAnswerer,
    allow_discord_reads: bool,
) -> Option<BrainOutcome> {
    match cooldowns.reserve_discord(user_id, channel_id).await {
        DiscordReservation::Accepted => {}
        DiscordReservation::DailyLimit => return Some(BrainOutcome::DailyLimit),
        DiscordReservation::Suppressed => return None,
    }
    Some(
        answer_discord_query_with_read_access(
            question,
            user_id,
            max_question_len,
            answerer,
            allow_discord_reads,
        )
        .await,
    )
}

pub async fn answer_discord_query(
    question: &str,
    user_id: u64,
    max_question_len: usize,
    answerer: &dyn AiAnswerer,
) -> BrainOutcome {
    answer_discord_query_with_read_access(question, user_id, max_question_len, answerer, true).await
}

pub async fn answer_discord_query_with_read_access(
    question: &str,
    user_id: u64,
    max_question_len: usize,
    answerer: &dyn AiAnswerer,
    allow_discord_reads: bool,
) -> BrainOutcome {
    let question = question.trim();
    if question.is_empty() {
        return BrainOutcome::Usage;
    }
    let len = question.chars().count();
    if len > max_question_len {
        return BrainOutcome::TooLong { len };
    }
    match answerer
        .answer_for_discord_with_read_access(question, user_id, allow_discord_reads)
        .await
    {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::warn!("Discord-Brain-Anfrage fehlgeschlagen");
            BrainOutcome::BackendError
        }
    }
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

    if user_id == 0 {
        return BrainOutcome::BackendError;
    }
    let outcome = match answerer.answer_for_discord(question, user_id).await {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::warn!("Gemeinsame Brain-Antwort fehlgeschlagen");
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

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("Deterministischer Testzeitpunkt")
            .with_timezone(&Utc)
    }

    #[test]
    fn discord_limit_laesst_fuenfzig_sofort_zu_und_meldet_die_einundfuenfzigste_einmal() {
        let mut state = DiscordRateState::default();
        let now = utc("2026-10-07T12:00:00Z");
        for channel in 1..=50 {
            assert_eq!(
                state.reserve(1, channel, now, 50),
                DiscordReservation::Accepted
            );
        }
        assert_eq!(state.users[&1].count, 50);
        assert_eq!(
            state.reserve(1, 51, now, 50),
            DiscordReservation::DailyLimit
        );
        for channel in 1..=100 {
            assert_eq!(
                state.reserve(1, channel, now, 50),
                DiscordReservation::Suppressed
            );
        }
        assert_eq!(state.users[&1].count, 50);
        assert_eq!(state.reserve(2, 1, now, 50), DiscordReservation::Accepted);
    }

    #[test]
    fn discord_limit_ist_konfigurierbar_ohne_kanal_stunden_oder_globales_tageslimit() {
        let mut state = DiscordRateState::default();
        let now = utc("2026-10-07T12:00:00Z");
        for user in 1..=501 {
            for _ in 0..2 {
                assert_eq!(state.reserve(user, 1, now, 2), DiscordReservation::Accepted);
            }
            assert_eq!(
                state.reserve(user, 2, now, 2),
                DiscordReservation::DailyLimit
            );
        }
        assert_eq!(state.users.len(), 501);
        for (user, channel) in [(0, 1), (1, 0)] {
            assert_eq!(
                state.reserve(user, channel, now, 2),
                DiscordReservation::Suppressed
            );
        }
    }

    #[test]
    fn discord_limit_wechselt_am_berliner_kalendertag_auch_bei_zeitumstellungen() {
        for (before, midnight, after_utc_midnight, transition_before, transition_after, next_day) in [
            (
                "2026-03-28T22:59:59Z",
                "2026-03-28T23:00:00Z",
                "2026-03-29T00:00:00Z",
                "2026-03-29T00:59:59Z",
                "2026-03-29T01:00:00Z",
                "2026-03-29T22:00:00Z",
            ),
            (
                "2026-10-24T21:59:59Z",
                "2026-10-24T22:00:00Z",
                "2026-10-25T00:00:00Z",
                "2026-10-25T00:59:59Z",
                "2026-10-25T01:00:00Z",
                "2026-10-25T23:00:00Z",
            ),
            (
                "2026-01-07T22:59:59Z",
                "2026-01-07T23:00:00Z",
                "2026-01-08T00:00:00Z",
                "2026-01-08T01:00:00Z",
                "2026-01-08T02:00:00Z",
                "2026-01-08T23:00:00Z",
            ),
        ] {
            let mut state = DiscordRateState::default();
            assert_eq!(
                state.reserve(1, 1, utc(before), 1),
                DiscordReservation::Accepted
            );
            assert_eq!(
                state.reserve(1, 1, utc(before), 1),
                DiscordReservation::DailyLimit
            );
            assert_eq!(
                state.reserve(1, 1, utc(midnight), 1),
                DiscordReservation::Accepted
            );
            assert_eq!(
                state.reserve(1, 1, utc(midnight), 1),
                DiscordReservation::DailyLimit
            );
            for time in [after_utc_midnight, transition_before, transition_after] {
                assert_eq!(
                    state.reserve(1, 2, utc(time), 1),
                    DiscordReservation::Suppressed
                );
            }
            assert_eq!(
                state.reserve(1, 1, utc(next_day), 1),
                DiscordReservation::Accepted
            );
            assert_eq!(
                state.reserve(1, 1, utc(next_day), 1),
                DiscordReservation::DailyLimit
            );
        }
    }

    #[tokio::test]
    async fn discord_verwendet_keinen_alten_antwortweg_als_ersatz() {
        let answerer = CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        let cooldowns = BrainCooldowns::new(1);
        assert_eq!(
            handle_discord_query("Frage", 3, 4, 300, &cooldowns, &answerer).await,
            Some(BrainOutcome::BackendError)
        );
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            handle_brain_query(
                "Frage",
                3,
                &BrainConfig {
                    max_question_len: 300,
                    cooldown_secs: 0,
                },
                &cooldowns,
                &answerer,
            )
            .await,
            BrainOutcome::BackendError
        );
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            handle_discord_query("Frage", 3, 4, 300, &cooldowns, &answerer).await,
            Some(BrainOutcome::DailyLimit)
        );
        assert!(
            handle_discord_query("Frage", 3, 5, 300, &cooldowns, &answerer)
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
        let limits = BrainCooldowns::new(1);
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
        assert_eq!(second, Some(BrainOutcome::DailyLimit));
        assert_eq!(limits.discord.lock().await.users[&3].count, 1);
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

    struct DiscordAnswerer(CountingAnswerer);

    type TestAnswerFuture<'a> = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<BrainOutcome, BrainError>> + Send + 'a>,
    >;

    impl AiAnswerer for DiscordAnswerer {
        fn answer<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _question: &'life1 str,
        ) -> TestAnswerFuture<'async_trait>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async {
                panic!("Discord-Kommandos dürfen keinen anonymen Antwortweg verwenden")
            })
        }

        fn answer_for_discord<'life0, 'life1, 'async_trait>(
            &'life0 self,
            question: &'life1 str,
            user_id: u64,
        ) -> TestAnswerFuture<'async_trait>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                assert_eq!(user_id, 1);
                self.0.answer(question).await
            })
        }
    }

    #[tokio::test]
    async fn private_lesesperre_faellt_nicht_auf_alten_discord_antwortweg_zurueck() {
        let answerer = DiscordAnswerer(CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let cooldowns = BrainCooldowns::default();
        assert_eq!(
            handle_discord_query_with_read_access(
                "Was macht Abrams?",
                1,
                1,
                300,
                &cooldowns,
                &answerer,
                false,
            )
            .await,
            Some(BrainOutcome::BackendError)
        );
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 0);
        assert_eq!(cooldowns.discord.lock().await.users[&1].count, 1);
        assert_eq!(
            handle_discord_query("Was macht Abrams?", 1, 1, 300, &cooldowns, &answerer).await,
            Some(BrainOutcome::Answer("Was macht Abrams?".into()))
        );
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 1);
        assert_eq!(cooldowns.discord.lock().await.users[&1].count, 2);
    }

    #[tokio::test]
    async fn discord_folgefragen_und_mehrfachfrage_zaehlen_je_einen_zentralen_aufruf() {
        let cooldowns = BrainCooldowns::default();
        let answerer = DiscordAnswerer(CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let question = "Was ist Abrams?\nWelche Items passen?\nWie spiele ich die Lane?";
        for channel in 1..=DEFAULT_DISCORD_DAILY_USER_LIMIT as u64 {
            assert_eq!(
                handle_discord_query(question, 1, channel, 300, &cooldowns, &answerer).await,
                Some(BrainOutcome::Answer(question.into()))
            );
        }
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 50);
        assert_eq!(cooldowns.discord.lock().await.users[&1].count, 50);
        assert_eq!(
            handle_discord_query(question, 1, 1, 300, &cooldowns, &answerer).await,
            Some(BrainOutcome::DailyLimit)
        );
        assert!(
            handle_discord_query(question, 1, 2, 300, &cooldowns, &answerer)
                .await
                .is_none()
        );
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 50);
    }

    #[tokio::test]
    async fn gemeinsame_antwort_erhaelt_frage_und_beachtet_grenzen_und_cooldown() {
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        let answerer = DiscordAnswerer(CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: false,
        });
        assert_eq!(
            handle_brain_query("", 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::Usage
        );
        assert_eq!(
            handle_brain_query(&"x".repeat(21), 1, &config, &cooldowns, &answerer).await,
            BrainOutcome::TooLong { len: 21 }
        );
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            handle_brain_query("Frage", 0, &config, &cooldowns, &answerer).await,
            BrainOutcome::BackendError
        );
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 0);
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
        assert_eq!(answerer.0.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn backendfehler_verbraucht_keinen_cooldown() {
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        let answerer = DiscordAnswerer(CountingAnswerer {
            calls: AtomicUsize::new(0),
            fail: true,
        });
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
