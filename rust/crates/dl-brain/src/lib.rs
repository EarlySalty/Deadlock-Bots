use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

pub const DISCORD_MESSAGE_LIMIT: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrainOutcome {
    Usage,
    TooLong { len: usize },
    Cooldown { remaining_secs: u64 },
    Answer(Vec<String>),
    OutOfDomain,
    NoAnswer,
    BackendError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainContext {
    pub intent: String,
    pub prompt: String,
    #[serde(default)]
    pub sources: Vec<String>,
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
}

#[async_trait::async_trait]
pub trait BrainRetriever: Send + Sync {
    async fn ask_context(&self, frage: &str) -> Result<BrainContext, BrainError>;
}

#[async_trait::async_trait]
pub trait AiAnswerer: Send + Sync {
    async fn answer(&self, prompt: &str) -> Result<Option<String>, BrainError>;
}

pub async fn handle_brain_query(
    question: &str,
    user_id: u64,
    cfg: &BrainConfig,
    cooldowns: &BrainCooldowns,
    retriever: &dyn BrainRetriever,
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

    let context = match retriever.ask_context(question).await {
        Ok(context) => context,
        Err(err) => {
            tracing::warn!(%err, "Brain-Retrieval fehlgeschlagen");
            return BrainOutcome::BackendError;
        }
    };

    if context.intent.trim() == "out_of_domain" {
        register_cooldown(user_id, cfg, cooldowns).await;
        return BrainOutcome::OutOfDomain;
    }

    if context.prompt.trim().is_empty() {
        tracing::warn!("Brain-Retrieval lieferte leeren Prompt");
        return BrainOutcome::BackendError;
    }

    let answer = match answerer.answer(&context.prompt).await {
        Ok(Some(answer)) => answer,
        Ok(None) => {
            register_cooldown(user_id, cfg, cooldowns).await;
            return BrainOutcome::NoAnswer;
        }
        Err(err) => {
            tracing::warn!(%err, "Brain-Antwort fehlgeschlagen");
            return BrainOutcome::BackendError;
        }
    };
    if answer.trim().is_empty() {
        register_cooldown(user_id, cfg, cooldowns).await;
        return BrainOutcome::NoAnswer;
    }

    let chunks = chunk_message(&answer, DISCORD_MESSAGE_LIMIT);
    let outcome = if chunks.is_empty() {
        BrainOutcome::NoAnswer
    } else {
        BrainOutcome::Answer(chunks)
    };
    register_cooldown(user_id, cfg, cooldowns).await;
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
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    use super::*;

    struct StaticRetriever {
        result: Result<BrainContext, BrainError>,
        seen_question: Arc<StdMutex<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl BrainRetriever for StaticRetriever {
        async fn ask_context(&self, frage: &str) -> Result<BrainContext, BrainError> {
            *self.seen_question.lock().expect("seen_question lock") = Some(frage.to_string());
            self.result.clone()
        }
    }

    struct CountingAnswerer {
        result: Option<String>,
        calls: AtomicUsize,
        seen_prompt: Arc<StdMutex<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl AiAnswerer for CountingAnswerer {
        async fn answer(&self, prompt: &str) -> Result<Option<String>, BrainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.seen_prompt.lock().expect("seen_prompt lock") = Some(prompt.to_string());
            Ok(self.result.clone())
        }
    }

    fn context(intent: &str, prompt: &str) -> BrainContext {
        BrainContext {
            intent: intent.to_string(),
            prompt: prompt.to_string(),
            sources: Vec::new(),
        }
    }

    fn retriever(result: Result<BrainContext, BrainError>) -> StaticRetriever {
        StaticRetriever {
            result,
            seen_question: Arc::new(StdMutex::new(None)),
        }
    }

    fn answerer(result: Option<&str>) -> CountingAnswerer {
        CountingAnswerer {
            result: result.map(str::to_string),
            calls: AtomicUsize::new(0),
            seen_prompt: Arc::new(StdMutex::new(None)),
        }
    }

    fn cfg() -> BrainConfig {
        BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        }
    }

    #[tokio::test]
    async fn leere_frage_liefert_usage() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("general", "prompt")));
        let answerer = answerer(Some("antwort"));

        let out = handle_brain_query(" \n\t ", 1, &cfg(), &cooldowns, &retriever, &answerer).await;

        assert_eq!(out, BrainOutcome::Usage);
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn zu_lange_frage_liefert_toolong() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("general", "prompt")));
        let answerer = answerer(Some("antwort"));

        let out = handle_brain_query(
            "123456789012345678901",
            1,
            &cfg(),
            &cooldowns,
            &retriever,
            &answerer,
        )
        .await;

        assert_eq!(out, BrainOutcome::TooLong { len: 21 });
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cooldown_blockt_zweiten_call_und_laeuft_ab() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("general", "prompt")));
        let answerer = answerer(Some("antwort"));

        let first =
            handle_brain_query("frage", 42, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert!(matches!(first, BrainOutcome::Answer(_)));

        let second =
            handle_brain_query("frage", 42, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert!(matches!(
            second,
            BrainOutcome::Cooldown {
                remaining_secs: 1..=20
            }
        ));

        cooldowns
            .lock()
            .await
            .insert(42, Instant::now() - Duration::from_secs(21));
        let third =
            handle_brain_query("frage", 42, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert!(matches!(third, BrainOutcome::Answer(_)));
    }

    #[tokio::test]
    async fn backend_error_setzt_keinen_cooldown() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Err(BrainError::Backend("kaputt".to_string())));
        let answerer = answerer(Some("antwort"));

        let first = handle_brain_query("frage", 7, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert_eq!(first, BrainOutcome::BackendError);
        assert!(cooldowns.lock().await.is_empty());

        let second =
            handle_brain_query("frage", 7, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert_eq!(second, BrainOutcome::BackendError);
    }

    #[tokio::test]
    async fn out_of_domain_ruft_ai_nicht_auf() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("out_of_domain", "prompt")));
        let answerer = answerer(Some("soll nicht passieren"));

        let out = handle_brain_query("wetter?", 1, &cfg(), &cooldowns, &retriever, &answerer).await;

        assert_eq!(out, BrainOutcome::OutOfDomain);
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn out_of_domain_setzt_cooldown() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("out_of_domain", "prompt")));
        let answerer = answerer(Some("soll nicht passieren"));

        let first =
            handle_brain_query("wetter?", 9, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert_eq!(first, BrainOutcome::OutOfDomain);

        let second =
            handle_brain_query("wetter?", 9, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert!(matches!(second, BrainOutcome::Cooldown { .. }));
    }

    #[tokio::test]
    async fn cooldown_setzen_prunt_abgelaufene_eintraege() {
        let cooldowns = BrainCooldowns::default();
        cooldowns
            .lock()
            .await
            .insert(99, Instant::now() - Duration::from_secs(21));
        let retriever = retriever(Ok(context("general", "prompt")));
        let answerer = answerer(Some("antwort"));

        let out = handle_brain_query("frage", 1, &cfg(), &cooldowns, &retriever, &answerer).await;
        assert!(matches!(out, BrainOutcome::Answer(_)));

        let map = cooldowns.lock().await;
        assert!(map.contains_key(&1));
        assert!(!map.contains_key(&99));
    }

    #[tokio::test]
    async fn normalpfad_liefert_answer_und_prompt_an_ai() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("build_recommendation", "brain prompt")));
        let seen_prompt = retriever.seen_question.clone();
        let answerer = answerer(Some("fertige antwort"));
        let answerer_prompt = answerer.seen_prompt.clone();

        let out =
            handle_brain_query("Seven build", 1, &cfg(), &cooldowns, &retriever, &answerer).await;

        assert_eq!(
            out,
            BrainOutcome::Answer(vec!["fertige antwort".to_string()])
        );
        assert_eq!(
            seen_prompt.lock().expect("seen_question lock").as_deref(),
            Some("Seven build")
        );
        assert_eq!(
            answerer_prompt.lock().expect("seen_prompt lock").as_deref(),
            Some("brain prompt")
        );
        assert_eq!(answerer.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ai_none_oder_leer_liefert_no_answer() {
        let cooldowns = BrainCooldowns::default();
        let retriever = retriever(Ok(context("general", "prompt")));
        let none_answerer = answerer(None);

        let out =
            handle_brain_query("frage", 1, &cfg(), &cooldowns, &retriever, &none_answerer).await;
        assert_eq!(out, BrainOutcome::NoAnswer);

        cooldowns.lock().await.clear();
        let empty_answerer = answerer(Some("   "));
        let out =
            handle_brain_query("frage", 1, &cfg(), &cooldowns, &retriever, &empty_answerer).await;
        assert_eq!(out, BrainOutcome::NoAnswer);
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
