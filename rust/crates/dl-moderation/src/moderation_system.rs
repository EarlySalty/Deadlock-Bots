use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;

use crate::action_policy::{ActionPolicy, ModerationAction, PolicyDecision, PolicyDecisionSource};
use crate::behavior_detector::{BehaviorDetector, BehaviorSignal, BehaviorTriggerType};
use crate::case_embed::{build_case_components, build_compact_case_embed, CompactCaseEmbedInput};
use crate::content_analyzer::{ContentModerationPipeline, ModerationInput};
use crate::moderation_channel::{DEFAULT_MODERATION_CHANNEL_ID, DEFAULT_SCAN_CHANNEL_IDS};
use crate::moderation_verdict::{
    ContentAnalysis, ModerationCategory, ModerationVerdict, VerificationDecision,
};
use crate::store::{CaseAttachment, CaseDraft, CaseRecord, ModerationStore};
use crate::ReviewOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationSystemConfig {
    pub scan_channel_ids: Vec<u64>,
    pub moderation_channel_id: u64,
    pub enforce: bool,
}

impl Default for ModerationSystemConfig {
    fn default() -> Self {
        Self {
            scan_channel_ids: DEFAULT_SCAN_CHANNEL_IDS.to_vec(),
            moderation_channel_id: DEFAULT_MODERATION_CHANNEL_ID,
            enforce: true,
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
    async fn unban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool;
    async fn post_moderation_case(
        &self,
        channel_id: u64,
        embed: Value,
        components: Value,
    ) -> Option<u64>;
}

#[async_trait]
pub trait ModerationCaseStore: Send + Sync {
    async fn insert_case(&self, draft: CaseDraft) -> Option<String>;
    async fn update_case_action(&self, case_id: &str, action: &str);
    async fn set_review_message(&self, case_id: &str, message_id: u64);
    async fn fetch_case(&self, case_id: &str) -> Option<CaseRecord>;
    async fn resolve_case(&self, case_id: &str, action: &str, mod_id: u64);
    async fn resolve_case_denied(&self, case_id: &str, mod_id: u64, reason: &str);
}

#[async_trait]
impl ModerationCaseStore for ModerationStore {
    async fn insert_case(&self, draft: CaseDraft) -> Option<String> {
        ModerationStore::insert_case(self, draft).await
    }

    async fn update_case_action(&self, case_id: &str, action: &str) {
        ModerationStore::update_case_action(self, case_id, action).await;
    }

    async fn set_review_message(&self, case_id: &str, message_id: u64) {
        ModerationStore::set_review_message(self, case_id, message_id).await;
    }

    async fn fetch_case(&self, case_id: &str) -> Option<CaseRecord> {
        ModerationStore::fetch_case(self, case_id).await
    }

    async fn resolve_case(&self, case_id: &str, action: &str, mod_id: u64) {
        ModerationStore::resolve_case(self, case_id, action, mod_id).await;
    }

    async fn resolve_case_denied(&self, case_id: &str, mod_id: u64, reason: &str) {
        ModerationStore::resolve_case_denied(self, case_id, mod_id, reason).await;
    }
}

pub struct ModerationSystem<S: ModerationCaseStore = ModerationStore> {
    pub store: S,
    pipeline: ContentModerationPipeline,
    behavior_detector: Option<Arc<BehaviorDetector>>,
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
        Self::new_with_behavior_detector(pool, pipeline, None, policy, port, config)
    }

    pub fn new_with_behavior_detector(
        pool: PgPool,
        pipeline: ContentModerationPipeline,
        behavior_detector: Option<Arc<BehaviorDetector>>,
        policy: ActionPolicy,
        port: Arc<dyn ModerationPort>,
        config: ModerationSystemConfig,
    ) -> Arc<Self> {
        Self::new_with_store(
            ModerationStore { pool },
            pipeline,
            behavior_detector,
            policy,
            port,
            config,
        )
    }
}

impl<S: ModerationCaseStore> ModerationSystem<S> {
    fn new_with_store(
        store: S,
        pipeline: ContentModerationPipeline,
        behavior_detector: Option<Arc<BehaviorDetector>>,
        policy: ActionPolicy,
        port: Arc<dyn ModerationPort>,
        config: ModerationSystemConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            pipeline,
            behavior_detector,
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
        if !event.author_staff_status_known {
            tracing::warn!(
                guild_id,
                user_id = event.author_id,
                "Moderation: Staff-Status nicht im Cache, fail-closed skip"
            );
            return;
        }
        if event.author_is_staff || event.author_is_admin || event.author_can_manage_messages {
            return;
        }
        if event.content.trim().is_empty() && event.attachment_count == 0 {
            return;
        }

        // Verhaltens-Erkennung (Takeover/Burst) laeuft serverweit: uebernommene Konten
        // koennen in jedem Kanal posten. Bei Verhaltenstreffern laeuft Content-AI als
        // Richter nach, damit reine Heuristiken keine Fake-Konfidenz erzeugen.
        let behavior_signal = if let Some(detector) = &self.behavior_detector {
            detector.detect(guild_id, event).await
        } else {
            None
        };
        let content_verdict = if (self.config.scan_channel_ids.contains(&event.channel_id)
            || behavior_signal.is_some())
            && !(event.content.trim().is_empty() && event.image_attachment_urls.is_empty())
        {
            let input =
                ModerationInput::new(event.content.clone(), event.image_attachment_urls.clone());
            self.pipeline.evaluate(&input).await
        } else {
            None
        };
        let outcome = self
            .policy
            .decide_combined_outcome(content_verdict.as_ref(), behavior_signal.as_ref());
        let decision = outcome.decision;
        if matches!(decision, PolicyDecision::Ignore) {
            return;
        };

        let verdict = match outcome.source {
            Some(PolicyDecisionSource::Behavior) => {
                let is_takeover = behavior_signal
                    .as_ref()
                    .map(|signal| signal.trigger_type == BehaviorTriggerType::AccountTakeover)
                    .unwrap_or(false);
                if is_takeover {
                    behavior_signal.as_ref().map(behavior_verdict)
                } else {
                    content_verdict
                }
            }
            Some(PolicyDecisionSource::Content) => content_verdict,
            None => content_verdict.or_else(|| behavior_signal.as_ref().map(behavior_verdict)),
        };
        let Some(verdict) = verdict else {
            return;
        };
        self.persist_execute_and_post(guild_id, event, verdict, behavior_signal, decision)
            .await;
    }

    async fn persist_execute_and_post(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: ModerationVerdict,
        behavior_signal: Option<BehaviorSignal>,
        decision: PolicyDecision,
    ) {
        let action = match decision {
            PolicyDecision::AutoExecute { .. } => "auto_execute",
            PolicyDecision::Proposal { .. } => "proposed",
            PolicyDecision::Ignore => "ignored",
        };
        let draft = self.case_draft(
            guild_id,
            event,
            &verdict,
            behavior_signal.as_ref(),
            action,
            &decision,
        );
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
        if let PolicyDecision::AutoExecute {
            action,
            timeout_minutes,
        } = decision
        {
            if self.config.enforce {
                let deleted = self
                    .port
                    .delete_message(
                        event.channel_id,
                        event.message_id,
                        "Automatische Moderation: Nachricht entfernt",
                    )
                    .await;
                executed_actions.push(format!("delete:{}", if deleted { "ok" } else { "failed" }));
                let action_ok = match action {
                    ModerationAction::Timeout => {
                        let timed_out = self
                            .port
                            .timeout_member(
                                guild_id,
                                event.author_id,
                                timeout_minutes,
                                "Automatische Moderation: Timeout",
                            )
                            .await;
                        executed_actions.push(format!(
                            "timeout:{}",
                            if timed_out { "ok" } else { "failed" }
                        ));
                        timed_out
                    }
                    ModerationAction::Ban => {
                        let banned = self
                            .port
                            .ban_member(guild_id, event.author_id, "Automatische Moderation: Bann")
                            .await;
                        executed_actions
                            .push(format!("ban:{}", if banned { "ok" } else { "failed" }));
                        banned
                    }
                };
                let final_action = match (action, deleted && action_ok) {
                    (ModerationAction::Timeout, true) => "auto_timeout",
                    (ModerationAction::Ban, true) => "auto_ban",
                    (ModerationAction::Timeout, false) => "auto_timeout_failed",
                    (ModerationAction::Ban, false) => "auto_ban_failed",
                };
                self.store.update_case_action(&case_id, final_action).await;
            } else {
                executed_actions.push("shadow:no_action".to_string());
                self.store
                    .update_case_action(&case_id, "auto_execute_shadow")
                    .await;
            }
        }

        let embed = build_compact_case_embed(&CompactCaseEmbedInput {
            guild_id,
            channel_id: event.channel_id,
            message_id: event.message_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            verdict,
            behavior_signal,
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
        behavior_signal: Option<&BehaviorSignal>,
        action: &str,
        policy_decision: &PolicyDecision,
    ) -> CaseDraft {
        let source = match behavior_signal {
            Some(signal)
                if verdict.trigger == signal.trigger_label()
                    && verdict.verification.reason == signal.reason_code =>
            {
                "behavior"
            }
            Some(_) => "content+behavior",
            None => "content",
        };
        let trigger_type = behavior_signal
            .map(|signal| signal.trigger_label().to_string())
            .or_else(|| Some("content".to_string()));
        CaseDraft {
            guild_id,
            channel_id: event.channel_id,
            message_id: event.message_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            content: event.content.clone(),
            category: effective_category_label(verdict, behavior_signal),
            confidence: verdict.verification.confidence,
            reason: verdict.verification.reason.clone(),
            action: action.to_string(),
            source: source.to_string(),
            trigger_type,
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
                "behavior": behavior_signal.map(behavior_signal_raw),
                "policy": {
                    "action": action,
                    "timeout_minutes": policy_timeout_minutes(policy_decision),
                },
            })
            .to_string(),
            timeout_minutes: policy_timeout_minutes(policy_decision),
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
                "Von Moderator bestätigt: Nachricht entfernt",
            )
            .await;
        let _timed_out = self
            .port
            .timeout_member(
                case.guild_id,
                case.user_id,
                case.timeout_minutes
                    .filter(|minutes| *minutes > 0)
                    .unwrap_or_else(|| self.policy.config().timeout_minutes),
                "Von Moderator bestätigt: Timeout",
            )
            .await;
        self.store.resolve_case(case_id, "accepted", mod_id).await;
        ReviewOutcome::Done("Übernommen — Nachricht gelöscht und Timeout gesetzt.".to_string())
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
                "Von Moderator bestätigt: Nachricht entfernt (Bann)",
            )
            .await;
        let _banned = self
            .port
            .ban_member(case.guild_id, case.user_id, "Von Moderator bestätigt: Bann")
            .await;
        self.store.resolve_case(case_id, "banned", mod_id).await;
        ReviewOutcome::Done("Gebannt — Nachricht gelöscht.".to_string())
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
        ReviewOutcome::Done("Verworfen — keine Aktion, Nachricht bleibt.".to_string())
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
                "Timeout durch Moderator aufgehoben",
            )
            .await;
        self.store
            .resolve_case(case_id, "timeout_reversed", mod_id)
            .await;
        ReviewOutcome::Done("Timeout aufgehoben.".to_string())
    }

    pub async fn unban_case(&self, case_id: &str, mod_id: u64) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if matches!(case.action.as_str(), "unbanned" | "denied") {
            return ReviewOutcome::AlreadyHandled;
        }
        let _ok = self
            .port
            .unban_member(
                case.guild_id,
                case.user_id,
                "Bann durch Moderator aufgehoben",
            )
            .await;
        self.store.resolve_case(case_id, "unbanned", mod_id).await;
        ReviewOutcome::Done("Entbannt.".to_string())
    }
}

fn behavior_verdict(signal: &BehaviorSignal) -> ModerationVerdict {
    let raw = behavior_signal_raw(signal).to_string();
    ModerationVerdict {
        analysis: ContentAnalysis {
            category: ModerationCategory::Other,
            confidence: 1.0,
            reason: signal.reason_code.clone(),
            raw_json: raw.clone(),
        },
        verification: VerificationDecision {
            confirmed: true,
            category: ModerationCategory::Other,
            confidence: 1.0,
            reason: signal.reason_code.clone(),
            raw_json: raw,
        },
        trigger: signal.trigger_label().to_string(),
    }
}

fn effective_category_label(
    verdict: &ModerationVerdict,
    behavior_signal: Option<&BehaviorSignal>,
) -> String {
    if let Some(signal) = behavior_signal {
        if verdict.trigger == signal.trigger_label()
            && verdict.verification.reason == signal.reason_code
        {
            return signal.trigger_label().to_string();
        }
    }
    verdict.verification.category.as_label().to_string()
}

fn policy_timeout_minutes(decision: &PolicyDecision) -> Option<i64> {
    match decision {
        PolicyDecision::AutoExecute {
            timeout_minutes, ..
        }
        | PolicyDecision::Proposal { timeout_minutes } => Some(*timeout_minutes),
        PolicyDecision::Ignore => None,
    }
}

fn behavior_signal_raw(signal: &BehaviorSignal) -> Value {
    serde_json::json!({
        "source": signal.source_label(),
        "trigger_type": signal.trigger_label(),
        "severity": signal.severity.as_label(),
        "action_hint": signal.action_hint.as_label(),
        "reason_code": &signal.reason_code,
        "account_is_new": signal.account_is_new,
        "evidence": {
            "window_seconds": signal.evidence.window_seconds,
            "channel_ids": &signal.evidence.channel_ids,
            "message_ids": &signal.evidence.message_ids,
            "message_count": signal.evidence.message_count,
            "attachment_count": signal.evidence.attachment_count,
            "image_count": signal.evidence.image_count,
            "keyword_hit": signal.evidence.keyword_hit,
            "account_age_hours": signal.evidence.account_age_hours,
            "join_age_hours": signal.evidence.join_age_hours,
            "invite_code": &signal.evidence.invite_code,
            "image_urls": &signal.evidence.image_urls,
        }
    })
}

fn case_already_handled(action: &str) -> bool {
    matches!(
        action,
        "accepted" | "denied" | "banned" | "timeout_reversed" | "unbanned"
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
    use crate::behavior_detector::BehaviorDetectorPort;
    use crate::content_analyzer::{ContentAnalyzer, ContentAnalyzerConfig};
    use crate::content_verifier::ContentVerifier;
    use sqlx::postgres::PgPoolOptions;
    use std::collections::HashMap;
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
        unbans: AtomicUsize,
        posts: AtomicUsize,
        timeout_minutes: Mutex<Vec<i64>>,
        posted_embeds: Mutex<Vec<Value>>,
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
            minutes: i64,
            _reason: &str,
        ) -> bool {
            self.timeouts.fetch_add(1, Ordering::Relaxed);
            self.timeout_minutes.lock().await.push(minutes);
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

        async fn unban_member(&self, _guild_id: u64, _user_id: u64, _reason: &str) -> bool {
            self.unbans.fetch_add(1, Ordering::Relaxed);
            true
        }

        async fn post_moderation_case(
            &self,
            _channel_id: u64,
            embed: Value,
            _components: Value,
        ) -> Option<u64> {
            self.posts.fetch_add(1, Ordering::Relaxed);
            self.posted_embeds.lock().await.push(embed);
            Some(55)
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        drafts: Mutex<Vec<CaseDraft>>,
        actions: Mutex<Vec<String>>,
        review_messages: Mutex<Vec<u64>>,
        records: Mutex<HashMap<String, CaseRecord>>,
    }

    #[async_trait::async_trait]
    impl ModerationCaseStore for MemoryStore {
        async fn insert_case(&self, draft: CaseDraft) -> Option<String> {
            let case_id = format!("case-{}", draft.message_id);
            self.records.lock().await.insert(
                case_id.clone(),
                CaseRecord {
                    case_id: case_id.clone(),
                    guild_id: draft.guild_id,
                    channel_id: draft.channel_id,
                    message_id: draft.message_id,
                    user_id: draft.user_id,
                    user_tag: draft.user_tag.clone(),
                    category: draft.category.clone(),
                    confidence: draft.confidence,
                    reason: draft.reason.clone(),
                    action: draft.action.clone(),
                    timeout_minutes: draft.timeout_minutes,
                },
            );
            self.drafts.lock().await.push(draft);
            Some(case_id)
        }

        async fn update_case_action(&self, case_id: &str, action: &str) {
            self.actions.lock().await.push(action.to_string());
            if let Some(record) = self.records.lock().await.get_mut(case_id) {
                record.action = action.to_string();
            }
        }

        async fn set_review_message(&self, _case_id: &str, message_id: u64) {
            self.review_messages.lock().await.push(message_id);
        }

        async fn fetch_case(&self, case_id: &str) -> Option<CaseRecord> {
            self.records.lock().await.get(case_id).cloned()
        }

        async fn resolve_case(&self, case_id: &str, action: &str, _mod_id: u64) {
            self.update_case_action(case_id, action).await;
        }

        async fn resolve_case_denied(&self, case_id: &str, _mod_id: u64, _reason: &str) {
            self.update_case_action(case_id, "denied").await;
        }
    }

    #[derive(Default)]
    struct FakeBehaviorPort;

    #[async_trait::async_trait]
    impl BehaviorDetectorPort for FakeBehaviorPort {
        async fn resolve_invite_guild(&self, _code: &str) -> Option<u64> {
            None
        }
    }

    struct StaticInviteBehaviorPort {
        guild_id: Option<u64>,
    }

    #[async_trait::async_trait]
    impl BehaviorDetectorPort for StaticInviteBehaviorPort {
        async fn resolve_invite_guild(&self, _code: &str) -> Option<u64> {
            self.guild_id
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
                enforce: true,
            },
        );
        (moderator, port)
    }

    async fn memory_moderator(
        analysis_responses: &[&str],
        verification_responses: &[&str],
        behavior_detector: Option<Arc<BehaviorDetector>>,
        scan_channel_ids: Vec<u64>,
        enforce: bool,
    ) -> (Arc<ModerationSystem<MemoryStore>>, Arc<CountingPort>) {
        let analyzer_text = Arc::new(StaticText::default());
        {
            let mut responses = analyzer_text.responses.lock().await;
            for response in analysis_responses {
                responses.push((*response).to_string());
            }
        }
        let verifier_text = Arc::new(StaticText::default());
        {
            let mut responses = verifier_text.responses.lock().await;
            for response in verification_responses {
                responses.push((*response).to_string());
            }
        }
        let port = Arc::new(CountingPort::default());
        let moderator = ModerationSystem::new_with_store(
            MemoryStore::default(),
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
            behavior_detector,
            ActionPolicy::new(ActionPolicyConfig::default()),
            port.clone(),
            ModerationSystemConfig {
                scan_channel_ids,
                moderation_channel_id: 99,
                enforce,
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

    fn image_event(
        user_id: u64,
        channel_id: u64,
        message_id: u64,
        created_at: i64,
        joined_at: Option<i64>,
    ) -> dl_discord::MessageEvent {
        let mut event = event_with_unpersistable_guild();
        event.guild_id = Some(1);
        event.author_id = user_id;
        event.channel_id = channel_id;
        event.message_id = message_id;
        event.content.clear();
        event.attachment_count = 1;
        event.image_attachment_count = 1;
        event.image_attachment_urls = vec![format!("https://img/{message_id}.png")];
        event.author_created_at = created_at;
        event.author_joined_at = joined_at;
        event
    }

    fn scanned_text_event(message_id: u64, content: &str) -> dl_discord::MessageEvent {
        let mut event = event_with_unpersistable_guild();
        event.guild_id = Some(1);
        event.message_id = message_id;
        event.content = content.to_string();
        event.author_created_at = chrono::Utc::now().timestamp() - 90 * 24 * 3600;
        event.author_joined_at = Some(chrono::Utc::now().timestamp() - 30 * 24 * 3600);
        event
    }

    fn burst_text_attachment_event(
        user_id: u64,
        channel_id: u64,
        message_id: u64,
        created_at: i64,
        joined_at: Option<i64>,
    ) -> dl_discord::MessageEvent {
        let mut event = scanned_text_event(message_id, "lfg wer hat bock auf ranked");
        event.author_id = user_id;
        event.channel_id = channel_id;
        event.message_created_at = chrono::Utc::now().timestamp();
        event.attachment_count = 1;
        event.image_attachment_count = 0;
        event.image_attachment_urls = Vec::new();
        event.author_created_at = created_at;
        event.author_joined_at = joined_at;
        event
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

    #[tokio::test]
    async fn shadow_mode_persists_and_posts_without_discord_enforcement_actions() {
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"scam","confidence":0.9,"reason":"Analyzer"}"#],
            &[r#"{"confirmed":true,"category":"scam","confidence":0.9,"reason":"Verifier"}"#],
            None,
            vec![42],
            false,
        )
        .await;

        moderator
            .handle_message(&scanned_text_event(101, "free crypto"))
            .await;

        assert_eq!(moderator.store.drafts.lock().await.len(), 1);
        assert_eq!(moderator.store.review_messages.lock().await.len(), 1);
        assert_eq!(port.posts.load(Ordering::Relaxed), 1);
        assert_eq!(port.deletes.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
        assert_eq!(port.bans.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeout_minutes.lock().await.as_slice(), &[] as &[i64]);
        assert_eq!(
            moderator.store.actions.lock().await.as_slice(),
            &["auto_execute_shadow".to_string()]
        );
    }

    #[tokio::test]
    async fn explicit_enforce_mode_executes_auto_action() {
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"scam","confidence":0.9,"reason":"Analyzer"}"#],
            &[r#"{"confirmed":true,"category":"scam","confidence":0.9,"reason":"Verifier"}"#],
            None,
            vec![42],
            true,
        )
        .await;

        moderator
            .handle_message(&scanned_text_event(102, "free crypto"))
            .await;

        assert_eq!(moderator.store.drafts.lock().await.len(), 1);
        assert_eq!(port.posts.load(Ordering::Relaxed), 1);
        assert_eq!(port.deletes.load(Ordering::Relaxed), 1);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 1);
        assert_eq!(port.bans.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeout_minutes.lock().await.as_slice(), &[1440]);
    }

    #[tokio::test]
    async fn weak_hero_player_trash_talk_does_not_create_review_case() {
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"harassment","confidence":0.55,"reason":"Analyzer"}"#],
            &[r#"{"confirmed":true,"category":"harassment","confidence":0.78,"reason":"Verifier"}"#],
            None,
            vec![42],
            true,
        )
        .await;

        moderator
            .handle_message(&scanned_text_event(
                103,
                "haze spieler benutzen nicht viel von ihrem gehirn das passt so",
            ))
            .await;

        assert_eq!(moderator.store.drafts.lock().await.len(), 0);
        assert_eq!(port.posts.load(Ordering::Relaxed), 0);
        assert_eq!(port.deletes.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn benign_burst_rate_does_not_create_case() {
        let detector = crate::behavior_detector::BehaviorDetector::new(Arc::new(FakeBehaviorPort));
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"game_related_ok","confidence":0.2,"reason":"harmlos"}"#],
            &[],
            Some(detector),
            vec![999],
            true,
        )
        .await;
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 100_000 * 3600;
        let joined_at = Some(now - 3_000 * 3600);

        moderator
            .handle_message(&burst_text_attachment_event(
                400, 10, 1300, created_at, joined_at,
            ))
            .await;
        moderator
            .handle_message(&burst_text_attachment_event(
                400, 11, 1301, created_at, joined_at,
            ))
            .await;

        assert!(moderator.store.drafts.lock().await.is_empty());
        assert_eq!(port.posts.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
        assert_eq!(port.bans.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn burst_rate_with_ai_confirmed_scam_creates_case_with_real_confidence() {
        let detector = crate::behavior_detector::BehaviorDetector::new(Arc::new(FakeBehaviorPort));
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"scam","confidence":0.7,"reason":"Krypto-Verdacht"}"#],
            &[r#"{"confirmed":true,"category":"scam","confidence":0.72,"reason":"Bestätigt"}"#],
            Some(detector),
            vec![999],
            true,
        )
        .await;
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 100_000 * 3600;
        let joined_at = Some(now - 3_000 * 3600);

        moderator
            .handle_message(&burst_text_attachment_event(
                401, 10, 1400, created_at, joined_at,
            ))
            .await;
        moderator
            .handle_message(&burst_text_attachment_event(
                401, 11, 1401, created_at, joined_at,
            ))
            .await;

        let drafts = moderator.store.drafts.lock().await;
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].category, "scam");
        assert_eq!(port.posts.load(Ordering::Relaxed), 1);
        let embed = port.posted_embeds.lock().await.pop().expect("embed");
        let serialized = embed.to_string();
        assert!(serialized.contains("72%"));
        assert!(!serialized.contains("100%"));
        assert!(serialized.contains("Nachrichten"));
        assert!(serialized.contains("Kanäle"));
        assert!(serialized.contains("scam"));
    }

    #[tokio::test]
    async fn takeover_signal_creates_one_case_embed_and_deletes_only_current_message() {
        let analyzer_text = Arc::new(StaticText::default());
        let verifier_text = Arc::new(StaticText::default());
        let port = Arc::new(CountingPort::default());
        let store = MemoryStore::default();
        let detector = crate::behavior_detector::BehaviorDetector::new(Arc::new(FakeBehaviorPort));
        let moderator = ModerationSystem::new_with_store(
            store,
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
            Some(detector),
            ActionPolicy::new(ActionPolicyConfig::default()),
            port.clone(),
            ModerationSystemConfig {
                scan_channel_ids: vec![10, 11],
                moderation_channel_id: 99,
                enforce: true,
            },
        );
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 10 * 3600;
        let joined_at = Some(now - 3600);

        moderator
            .handle_message(&image_event(200, 10, 1000, created_at, joined_at))
            .await;
        moderator
            .handle_message(&image_event(200, 11, 1001, created_at, joined_at))
            .await;

        assert_eq!(moderator.store.drafts.lock().await.len(), 1);
        assert_eq!(moderator.store.review_messages.lock().await.len(), 1);
        assert_eq!(port.posts.load(Ordering::Relaxed), 1);
        assert_eq!(port.deletes.load(Ordering::Relaxed), 1);
        assert_eq!(port.bans.load(Ordering::Relaxed), 1);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
        let draft = moderator.store.drafts.lock().await.pop().expect("draft");
        assert_eq!(draft.source, "behavior");
        assert_eq!(draft.trigger_type.as_deref(), Some("account_takeover"));
        assert!(draft.ai_raw_json.contains("account_takeover"));
    }

    #[tokio::test]
    async fn behavior_trigger_wins_persisted_and_embedded_verdict_when_it_decides_action() {
        let detector = crate::behavior_detector::BehaviorDetector::new(Arc::new(FakeBehaviorPort));
        let (moderator, port) = memory_moderator(
            &[
                r#"{"category":"other","confidence":0.9,"reason":"content-other"}"#,
                r#"{"category":"other","confidence":0.9,"reason":"content-other"}"#,
            ],
            &[
                r#"{"confirmed":false,"category":"other","confidence":0.9,"reason":"content-unconfirmed"}"#,
                r#"{"confirmed":false,"category":"other","confidence":0.9,"reason":"content-unconfirmed"}"#,
            ],
            Some(detector),
            vec![10, 11],
            false,
        )
        .await;
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 10 * 3600;
        let joined_at = Some(now - 3600);
        let mut first = image_event(300, 10, 1100, created_at, joined_at);
        first.content = "ambiguous report".to_string();
        let mut second = image_event(300, 11, 1101, created_at, joined_at);
        second.content = "ambiguous report".to_string();

        moderator.handle_message(&first).await;
        moderator.handle_message(&second).await;

        let draft = moderator.store.drafts.lock().await.pop().expect("draft");
        assert_eq!(draft.category, "account_takeover");
        assert_eq!(draft.reason, "behavior:account_takeover");
        let embed = port.posted_embeds.lock().await.pop().expect("embed");
        let serialized = embed.to_string();
        assert!(serialized.contains("account_takeover"));
        assert!(serialized.contains("behavior:account_takeover"));
        assert!(!serialized.contains("content-unconfirmed"));
    }

    #[tokio::test]
    async fn accept_uses_timeout_minutes_from_behavior_proposal() {
        let detector =
            crate::behavior_detector::BehaviorDetector::new(Arc::new(StaticInviteBehaviorPort {
                guild_id: Some(2),
            }));
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"other","confidence":0.7,"reason":"Invite-Kontext"}"#],
            &[r#"{"confirmed":true,"category":"other","confidence":0.72,"reason":"Bestätigt"}"#],
            Some(detector),
            vec![42],
            true,
        )
        .await;

        moderator
            .handle_message(&scanned_text_event(1200, "join https://discord.gg/FOREIGN"))
            .await;
        let outcome = moderator.accept_case("case-1200", 999).await;

        assert!(matches!(outcome, ReviewOutcome::Done(_)));
        assert_eq!(port.deletes.load(Ordering::Relaxed), 1);
        assert_eq!(port.timeout_minutes.lock().await.as_slice(), &[60]);
    }

    #[tokio::test]
    async fn behavior_takeover_fires_outside_scan_channels() {
        // Kontoübernahme kann in JEDEM Kanal posten. Die Verhaltens-Erkennung muss
        // serverweit greifen, auch wenn der Kanal nicht in scan_channel_ids steht.
        let analyzer_text = Arc::new(StaticText::default());
        let verifier_text = Arc::new(StaticText::default());
        let port = Arc::new(CountingPort::default());
        let store = MemoryStore::default();
        let detector = crate::behavior_detector::BehaviorDetector::new(Arc::new(FakeBehaviorPort));
        let moderator = ModerationSystem::new_with_store(
            store,
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
            Some(detector),
            ActionPolicy::new(ActionPolicyConfig::default()),
            port.clone(),
            ModerationSystemConfig {
                // Nachrichten laufen in Kanal 10/11 — bewusst NICHT in scan_channel_ids.
                scan_channel_ids: vec![777],
                moderation_channel_id: 99,
                enforce: true,
            },
        );
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 10 * 3600;
        let joined_at = Some(now - 3600);

        moderator
            .handle_message(&image_event(200, 10, 1000, created_at, joined_at))
            .await;
        moderator
            .handle_message(&image_event(200, 11, 1001, created_at, joined_at))
            .await;

        assert_eq!(port.bans.load(Ordering::Relaxed), 1);
        assert_eq!(port.deletes.load(Ordering::Relaxed), 1);
        assert_eq!(port.posts.load(Ordering::Relaxed), 1);
        let draft = moderator.store.drafts.lock().await.pop().expect("draft");
        assert_eq!(draft.source, "behavior");
        assert_eq!(draft.trigger_type.as_deref(), Some("account_takeover"));
    }

    #[tokio::test]
    async fn content_scan_stays_limited_to_scan_channels() {
        // Ohne Verhaltens-Detektor bleibt nur der Content-Pfad — der darf außerhalb
        // der scan_channel_ids NICHT feuern (LLM-Scan bleibt gezielt).
        let (moderator, port) = memory_moderator(
            &[r#"{"category":"scam","confidence":0.9,"reason":"Analyzer"}"#],
            &[r#"{"confirmed":true,"category":"scam","confidence":0.9,"reason":"Verifier"}"#],
            None,
            vec![777],
            true,
        )
        .await;

        // scanned_text_event postet in Kanal 42 — nicht in scan_channel_ids [777].
        moderator
            .handle_message(&scanned_text_event(500, "free crypto"))
            .await;

        assert_eq!(port.deletes.load(Ordering::Relaxed), 0);
        assert_eq!(port.timeouts.load(Ordering::Relaxed), 0);
        assert_eq!(port.posts.load(Ordering::Relaxed), 0);
        assert_eq!(moderator.store.drafts.lock().await.len(), 0);
    }
}
