use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;

use crate::action_policy::{ActionPolicy, PolicyDecision};
use crate::case_embed::{build_case_components, build_compact_case_embed, CompactCaseEmbedInput};
use crate::content_analyzer::{ContentModerationPipeline, ModerationInput};
use crate::moderation_channel::{DEFAULT_MODERATION_CHANNEL_ID, DEFAULT_SCAN_CHANNEL_IDS};
use crate::moderation_verdict::ModerationVerdict;
use crate::store::{CaseAttachment, CaseDraft, ModerationStore};
use crate::ReviewOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationSystemConfig {
    pub scan_channel_ids: Vec<u64>,
    pub moderation_channel_id: u64,
}

impl Default for ModerationSystemConfig {
    fn default() -> Self {
        Self {
            scan_channel_ids: DEFAULT_SCAN_CHANNEL_IDS.to_vec(),
            moderation_channel_id: DEFAULT_MODERATION_CHANNEL_ID,
        }
    }
}

#[async_trait]
pub trait ModerationPort: Send + Sync {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool;
    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64, reason: &str)
        -> bool;
    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool;
    async fn untimeout_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool;
    async fn post_moderation_case(
        &self,
        channel_id: u64,
        embed: Value,
        components: Value,
    ) -> Option<u64>;
}

pub struct ModerationSystem {
    pub store: ModerationStore,
    pipeline: ContentModerationPipeline,
    policy: ActionPolicy,
    port: Arc<dyn ModerationPort>,
    config: ModerationSystemConfig,
}

impl ModerationSystem {
    pub fn new(
        pool: PgPool,
        pipeline: ContentModerationPipeline,
        policy: ActionPolicy,
        port: Arc<dyn ModerationPort>,
        config: ModerationSystemConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: ModerationStore { pool },
            pipeline,
            policy,
            port,
            config,
        })
    }

    pub fn config(&self) -> &ModerationSystemConfig {
        &self.config
    }

    pub async fn handle_message(self: &Arc<Self>, event: &dl_discord::MessageEvent) {
        let Some(guild_id) = event.guild_id else {
            return;
        };
        if !self.config.scan_channel_ids.contains(&event.channel_id) {
            return;
        }
        if event.author_is_staff || event.author_is_admin || event.author_can_manage_messages {
            return;
        }
        if event.content.trim().is_empty() && event.image_attachment_urls.is_empty() {
            return;
        }

        let input =
            ModerationInput::new(event.content.clone(), event.image_attachment_urls.clone());
        let Some(verdict) = self.pipeline.evaluate(&input).await else {
            return;
        };
        let decision = self.policy.decide(&verdict);
        if matches!(decision, PolicyDecision::Ignore) {
            return;
        }
        self.persist_execute_and_post(guild_id, event, verdict, decision)
            .await;
    }

    async fn persist_execute_and_post(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: ModerationVerdict,
        decision: PolicyDecision,
    ) {
        let action = match decision {
            PolicyDecision::AutoExecute { .. } => "auto_execute",
            PolicyDecision::Proposal { .. } => "proposed",
            PolicyDecision::Ignore => "ignored",
        };
        let draft = self.case_draft(guild_id, event, &verdict, action);
        let Some(case_id) = self.store.insert_case(draft).await else {
            tracing::warn!(
                guild_id,
                channel_id = event.channel_id,
                message_id = event.message_id,
                user_id = event.author_id,
                action,
                "Moderation: Case konnte nicht persistiert werden; Aktion/Review-Post werden uebersprungen"
            );
            return;
        };

        let mut executed_actions = Vec::new();
        if let PolicyDecision::AutoExecute { timeout_minutes } = decision {
            let deleted = self
                .port
                .delete_message(
                    event.channel_id,
                    event.message_id,
                    "PLATZHALTER: Auto-Delete-Audit-Reason",
                )
                .await;
            let timed_out = self
                .port
                .timeout_member(
                    guild_id,
                    event.author_id,
                    timeout_minutes,
                    "PLATZHALTER: Auto-Timeout-Audit-Reason",
                )
                .await;
            executed_actions.push(format!("delete:{}", if deleted { "ok" } else { "failed" }));
            executed_actions.push(format!(
                "timeout:{}",
                if timed_out { "ok" } else { "failed" }
            ));
            let final_action = if deleted && timed_out {
                "auto_execute"
            } else {
                "auto_execute_failed"
            };
            self.store.update_case_action(&case_id, final_action).await;
        }

        let embed = build_compact_case_embed(&CompactCaseEmbedInput {
            guild_id,
            channel_id: event.channel_id,
            message_id: event.message_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            verdict,
            policy_decision: decision.clone(),
            executed_actions,
        });
        let components = build_case_components(&case_id, &decision);
        if let Some(message_id) = self
            .port
            .post_moderation_case(self.config.moderation_channel_id, embed, components)
            .await
        {
            self.store.set_review_message(&case_id, message_id).await;
        }
    }

    fn case_draft(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: &ModerationVerdict,
        action: &str,
    ) -> CaseDraft {
        CaseDraft {
            guild_id,
            channel_id: event.channel_id,
            message_id: event.message_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            content: event.content.clone(),
            category: verdict.verification.category.as_label().to_string(),
            confidence: verdict.verification.confidence,
            reason: verdict.verification.reason.clone(),
            action: action.to_string(),
            attachments: event
                .attachments
                .iter()
                .map(|attachment| CaseAttachment {
                    url: attachment.url.clone(),
                    content_type: attachment.content_type.clone(),
                    filename: attachment.filename.clone(),
                })
                .collect(),
            ai_raw_json: serde_json::json!({
                "analysis": {
                    "category": verdict.analysis.category.as_label(),
                    "confidence": verdict.analysis.confidence,
                    "reason": verdict.analysis.reason,
                    "raw": verdict.analysis.raw_json,
                },
                "verification": {
                    "confirmed": verdict.verification.confirmed,
                    "category": verdict.verification.category.as_label(),
                    "confidence": verdict.verification.confidence,
                    "reason": verdict.verification.reason,
                    "raw": verdict.verification.raw_json,
                },
                "trigger": verdict.trigger,
            })
            .to_string(),
            escalated_with_context: false,
        }
    }

    pub async fn accept_case(&self, case_id: &str, mod_id: u64) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if case_already_handled(&case.action) {
            return ReviewOutcome::AlreadyHandled;
        }
        let _deleted = self
            .port
            .delete_message(
                case.channel_id,
                case.message_id,
                "PLATZHALTER: Accept-Delete-Audit-Reason",
            )
            .await;
        let _timed_out = self
            .port
            .timeout_member(
                case.guild_id,
                case.user_id,
                self.policy.config().timeout_minutes,
                "PLATZHALTER: Accept-Timeout-Audit-Reason",
            )
            .await;
        self.store.resolve_case(case_id, "accepted", mod_id).await;
        ReviewOutcome::Done("PLATZHALTER: Accept-Reply".to_string())
    }

    pub async fn ban_case(&self, case_id: &str, mod_id: u64) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if case_already_handled(&case.action) {
            return ReviewOutcome::AlreadyHandled;
        }
        let _deleted = self
            .port
            .delete_message(
                case.channel_id,
                case.message_id,
                "PLATZHALTER: Ban-Delete-Audit-Reason",
            )
            .await;
        let _banned = self
            .port
            .ban_member(case.guild_id, case.user_id, "PLATZHALTER: Ban-Audit-Reason")
            .await;
        self.store.resolve_case(case_id, "banned", mod_id).await;
        ReviewOutcome::Done("PLATZHALTER: Ban-Reply".to_string())
    }

    pub async fn deny_case(&self, case_id: &str, mod_id: u64, reason: &str) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if case_already_handled(&case.action) {
            return ReviewOutcome::AlreadyHandled;
        }
        self.store
            .resolve_case_denied(case_id, mod_id, reason)
            .await;
        ReviewOutcome::Done("PLATZHALTER: Deny-Reply".to_string())
    }

    pub async fn untimeout_case(&self, case_id: &str, mod_id: u64) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if matches!(case.action.as_str(), "timeout_reversed" | "denied") {
            return ReviewOutcome::AlreadyHandled;
        }
        let _ok = self
            .port
            .untimeout_member(
                case.guild_id,
                case.user_id,
                "PLATZHALTER: Timeout-Reversal-Audit-Reason",
            )
            .await;
        self.store
            .resolve_case(case_id, "timeout_reversed", mod_id)
            .await;
        ReviewOutcome::Done("PLATZHALTER: Timeout-Reversal-Reply".to_string())
    }
}

fn case_already_handled(action: &str) -> bool {
    matches!(
        action,
        "accepted" | "denied" | "banned" | "timeout_reversed"
    )
}

pub fn spawn(
    moderator: Arc<ModerationSystem>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => moderator.handle_message(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_policy::ActionPolicyConfig;
    use crate::content_analyzer::{ContentAnalyzer, ContentAnalyzerConfig};
    use crate::content_verifier::ContentVerifier;
    use sqlx::postgres::PgPoolOptions;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct StaticText {
        responses: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl dl_ai::TextGenerator for StaticText {
        async fn generate_text(&self, _request: dl_ai::GenerateRequest) -> Option<String> {
            self.responses.lock().await.pop()
        }
    }

    #[derive(Default)]
    struct CountingPort {
        deletes: AtomicUsize,
        timeouts: AtomicUsize,
        bans: AtomicUsize,
        untimeouts: AtomicUsize,
        posts: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ModerationPort for CountingPort {
        async fn delete_message(&self, _channel_id: u64, _message_id: u64, _reason: &str) -> bool {
            self.deletes.fetch_add(1, Ordering::Relaxed);
            true
        }

        async fn timeout_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _minutes: i64,
            _reason: &str,
        ) -> bool {
            self.timeouts.fetch_add(1, Ordering::Relaxed);
            true
        }

        async fn ban_member(&self, _guild_id: u64, _user_id: u64, _reason: &str) -> bool {
            self.bans.fetch_add(1, Ordering::Relaxed);
            true
        }

        async fn untimeout_member(&self, _guild_id: u64, _user_id: u64, _reason: &str) -> bool {
            self.untimeouts.fetch_add(1, Ordering::Relaxed);
            true
        }

        async fn post_moderation_case(
            &self,
            _channel_id: u64,
            _embed: Value,
            _components: Value,
        ) -> Option<u64> {
            self.posts.fetch_add(1, Ordering::Relaxed);
            Some(55)
        }
    }

    fn lazy_pool() -> PgPool {
        PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/deadlock")
            .expect("lazy pg pool")
    }

    async fn moderator_with_responses(
        analysis: &str,
        verification: &str,
    ) -> (Arc<ModerationSystem>, Arc<CountingPort>) {
        let analyzer_text = Arc::new(StaticText::default());
        analyzer_text
            .responses
            .lock()
            .await
            .push(analysis.to_string());
        let verifier_text = Arc::new(StaticText::default());
        verifier_text
            .responses
            .lock()
            .await
            .push(verification.to_string());
        let port = Arc::new(CountingPort::default());
        let moderator = ModerationSystem::new(
            lazy_pool(),
            ContentModerationPipeline::new(
                ContentAnalyzer::new(
                    analyzer_text,
                    None,
                    ContentAnalyzerConfig {
                        text_model: "MiniMax-M3".to_string(),
                        image_model: "gpt-5.4-nano".to_string(),
                    },
                ),
                ContentVerifier::new(verifier_text, None, Default::default()),
                0.5,
            ),
            ActionPolicy::new(ActionPolicyConfig::default()),
            port.clone(),
            ModerationSystemConfig {
                scan_channel_ids: vec![42],
                moderation_channel_id: 99,
            },
        );
        (moderator, port)
    }

    fn event_with_unpersistable_guild() -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id: Some(i64::MAX as u64 + 1),
            channel_id: 42,
            message_id: 100,
            author_id: 200,
            author_display_name: "Anna".into(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: "free crypto".into(),
            message_created_at: 1_000,
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    #[test]
    fn default_config_uses_single_moderation_channel() {
        let config = ModerationSystemConfig::default();

        assert_eq!(config.moderation_channel_id, DEFAULT_MODERATION_CHANNEL_ID);
        assert_eq!(config.scan_channel_ids, DEFAULT_SCAN_CHANNEL_IDS);
    }

    #[tokio::test]
    async fn auto_execute_does_not_delete_or_timeout_when_case_persist_fails() {
        let (moderator, port) = moderator_with_responses(
            r#"{"category":"scam","confidence":0.9,"reason":"Analyzer"}"#,
            r#"{"confirmed":true,"category":"scam","confidence":0.9,"reason":"Verifier"}"#,
        )
        .await;

        moderator
            .handle_message(&event_with_unpersistable_guild())
            .await;

        assert_eq!(port.deletes.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
        assert_eq!(port.posts.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn proposal_does_not_post_review_with_fake_case_when_case_persist_fails() {
        let (moderator, port) = moderator_with_responses(
            r#"{"category":"harassment","confidence":0.9,"reason":"Analyzer"}"#,
            r#"{"confirmed":true,"category":"harassment","confidence":0.9,"reason":"Verifier"}"#,
        )
        .await;

        moderator
            .handle_message(&event_with_unpersistable_guild())
            .await;

        assert_eq!(port.deletes.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
        assert_eq!(port.posts.load(Ordering::Relaxed), 0);
    }
}
