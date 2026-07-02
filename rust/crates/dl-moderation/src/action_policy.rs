use crate::behavior_detector::{BehaviorActionHint, BehaviorSignal, BehaviorTriggerType};
use crate::moderation_verdict::ModerationVerdict;

#[derive(Debug, Clone, PartialEq)]
pub struct ActionPolicyConfig {
    pub auto_execute_verified_confidence: f64,
    pub proposal_verified_confidence: f64,
    pub timeout_minutes: i64,
    pub behavior_proposal_timeout_minutes: i64,
}

impl Default for ActionPolicyConfig {
    fn default() -> Self {
        Self {
            auto_execute_verified_confidence: 0.85,
            proposal_verified_confidence: 0.60,
            timeout_minutes: 1440,
            behavior_proposal_timeout_minutes: 60,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationAction {
    Timeout,
    Ban,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Ignore,
    AutoExecute {
        action: ModerationAction,
        timeout_minutes: i64,
    },
    Proposal {
        timeout_minutes: i64,
    },
}

#[derive(Debug, Clone)]
pub struct ActionPolicy {
    config: ActionPolicyConfig,
}

impl ActionPolicy {
    pub fn new(config: ActionPolicyConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ActionPolicyConfig {
        &self.config
    }

    pub fn decide(&self, verdict: &ModerationVerdict) -> PolicyDecision {
        self.decide_content(verdict)
    }

    pub fn decide_combined(
        &self,
        content: Option<&ModerationVerdict>,
        behavior: Option<&BehaviorSignal>,
    ) -> PolicyDecision {
        let content_decision = content
            .map(|verdict| self.decide_content(verdict))
            .unwrap_or(PolicyDecision::Ignore);
        let behavior_decision = behavior
            .map(|signal| self.decide_behavior(signal, content))
            .unwrap_or(PolicyDecision::Ignore);

        choose_strongest(content_decision, behavior_decision)
    }

    fn decide_content(&self, verdict: &ModerationVerdict) -> PolicyDecision {
        if !verdict.verification.confirmed
            || verdict.verification.confidence < self.config.proposal_verified_confidence
        {
            return PolicyDecision::Ignore;
        }

        if verdict.verification.category.is_high_damage()
            && verdict.verification.confidence >= self.config.auto_execute_verified_confidence
        {
            return PolicyDecision::AutoExecute {
                action: ModerationAction::Timeout,
                timeout_minutes: self.config.timeout_minutes,
            };
        }

        PolicyDecision::Proposal {
            timeout_minutes: self.config.timeout_minutes,
        }
    }

    fn decide_behavior(
        &self,
        signal: &BehaviorSignal,
        content: Option<&ModerationVerdict>,
    ) -> PolicyDecision {
        if signal.trigger_type == BehaviorTriggerType::AccountTakeover {
            let action = match signal.action_hint {
                BehaviorActionHint::Ban => ModerationAction::Ban,
                BehaviorActionHint::Timeout | BehaviorActionHint::Proposal => {
                    ModerationAction::Timeout
                }
            };
            return PolicyDecision::AutoExecute {
                action,
                timeout_minutes: self.config.timeout_minutes,
            };
        }

        if content.is_some_and(|verdict| {
            verdict.verification.confirmed
                && verdict.verification.category.is_high_damage()
                && verdict.verification.confidence >= self.config.auto_execute_verified_confidence
        }) {
            return PolicyDecision::AutoExecute {
                action: if signal.account_is_new {
                    ModerationAction::Ban
                } else {
                    ModerationAction::Timeout
                },
                timeout_minutes: self.config.timeout_minutes,
            };
        }

        PolicyDecision::Proposal {
            timeout_minutes: self.config.behavior_proposal_timeout_minutes,
        }
    }
}

fn choose_strongest(left: PolicyDecision, right: PolicyDecision) -> PolicyDecision {
    if decision_rank(&right) > decision_rank(&left) {
        right
    } else {
        left
    }
}

fn decision_rank(decision: &PolicyDecision) -> u8 {
    match decision {
        PolicyDecision::Ignore => 0,
        PolicyDecision::Proposal { .. } => 1,
        PolicyDecision::AutoExecute {
            action: ModerationAction::Timeout,
            ..
        } => 2,
        PolicyDecision::AutoExecute {
            action: ModerationAction::Ban,
            ..
        } => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior_detector::{BehaviorEvidence, BehaviorSeverity, BehaviorTriggerType};
    use crate::moderation_verdict::{
        parse_verification_decision, ContentAnalysis, ModerationCategory, ModerationVerdict,
        VerificationDecision,
    };

    fn verdict(category: ModerationCategory, verify_confidence: f64) -> ModerationVerdict {
        ModerationVerdict {
            analysis: ContentAnalysis {
                category: category.clone(),
                confidence: 0.91,
                reason: "Analyzer".to_string(),
                raw_json: "{}".to_string(),
            },
            verification: VerificationDecision {
                confirmed: true,
                category,
                confidence: verify_confidence,
                reason: "Verifier".to_string(),
                raw_json: "{}".to_string(),
            },
            trigger: "Text".to_string(),
        }
    }

    #[test]
    fn policy_auto_executes_only_high_damage_high_confidence() {
        let policy = ActionPolicy::new(ActionPolicyConfig::default());

        assert_eq!(
            policy.decide(&verdict(ModerationCategory::Scam, 0.86)),
            PolicyDecision::AutoExecute {
                action: ModerationAction::Timeout,
                timeout_minutes: 1440
            }
        );
        assert_eq!(
            policy.decide(&verdict(ModerationCategory::Csam, 0.85)),
            PolicyDecision::AutoExecute {
                action: ModerationAction::Timeout,
                timeout_minutes: 1440
            }
        );
        assert_eq!(
            policy.decide(&verdict(ModerationCategory::NsfwExplicit, 0.84)),
            PolicyDecision::Proposal {
                timeout_minutes: 1440
            }
        );
    }

    #[test]
    fn policy_proposes_non_high_damage_categories() {
        let policy = ActionPolicy::new(ActionPolicyConfig::default());

        assert_eq!(
            policy.decide(&verdict(ModerationCategory::Harassment, 0.99)),
            PolicyDecision::Proposal {
                timeout_minutes: 1440
            }
        );
    }

    #[test]
    fn policy_ignores_unconfirmed_or_below_verify_threshold() {
        let policy = ActionPolicy::new(ActionPolicyConfig::default());
        let mut case = verdict(ModerationCategory::Scam, 0.59);
        assert_eq!(policy.decide(&case), PolicyDecision::Ignore);

        case.verification.confirmed = false;
        case.verification.confidence = 0.99;
        assert_eq!(policy.decide(&case), PolicyDecision::Ignore);
    }

    #[test]
    fn verifier_without_explicit_category_cannot_auto_execute_suspected_high_damage() {
        let verification = parse_verification_decision(
            Some(r#"{"confirmed":true,"confidence":0.9,"reason":"Bestaetigt"}"#),
            ModerationCategory::Scam,
        );
        let case = ModerationVerdict {
            analysis: ContentAnalysis {
                category: ModerationCategory::Scam,
                confidence: 0.91,
                reason: "Analyzer".to_string(),
                raw_json: "{}".to_string(),
            },
            verification,
            trigger: "Text".to_string(),
        };

        assert_eq!(case.verification.category, ModerationCategory::Other);
        assert_eq!(
            ActionPolicy::new(ActionPolicyConfig::default()).decide(&case),
            PolicyDecision::Proposal {
                timeout_minutes: 1440
            }
        );
    }

    fn behavior_signal(
        trigger_type: BehaviorTriggerType,
        action_hint: BehaviorActionHint,
        account_is_new: bool,
    ) -> BehaviorSignal {
        BehaviorSignal {
            trigger_type,
            severity: if trigger_type == BehaviorTriggerType::AccountTakeover {
                BehaviorSeverity::Critical
            } else {
                BehaviorSeverity::Suspicious
            },
            action_hint,
            reason_code: format!("behavior:{}", trigger_type.as_label()),
            account_is_new,
            evidence: BehaviorEvidence {
                window_seconds: 30,
                channel_ids: vec![1, 2],
                message_ids: vec![10, 11],
                message_count: 2,
                attachment_count: 2,
                image_count: 2,
                keyword_hit: false,
                account_age_hours: 10,
                join_age_hours: Some(1),
                invite_code: None,
                image_urls: vec!["https://img/1.png".to_string()],
            },
            messages: Vec::new(),
        }
    }

    #[test]
    fn account_takeover_maps_to_auto_execute_via_unified_policy() {
        let policy = ActionPolicy::new(ActionPolicyConfig::default());

        assert_eq!(
            policy.decide_combined(
                None,
                Some(&behavior_signal(
                    BehaviorTriggerType::AccountTakeover,
                    BehaviorActionHint::Ban,
                    true,
                )),
            ),
            PolicyDecision::AutoExecute {
                action: ModerationAction::Ban,
                timeout_minutes: 1440,
            }
        );
        assert_eq!(
            policy.decide_combined(
                None,
                Some(&behavior_signal(
                    BehaviorTriggerType::AccountTakeover,
                    BehaviorActionHint::Timeout,
                    false,
                )),
            ),
            PolicyDecision::AutoExecute {
                action: ModerationAction::Timeout,
                timeout_minutes: 1440,
            }
        );
    }

    #[test]
    fn content_behavior_combination_keeps_heaviest_action() {
        let policy = ActionPolicy::new(ActionPolicyConfig::default());
        let content = verdict(ModerationCategory::Scam, 0.90);
        let weak_new_behavior = behavior_signal(
            BehaviorTriggerType::YoungAccountBurst,
            BehaviorActionHint::Proposal,
            true,
        );

        assert_eq!(
            policy.decide_combined(Some(&content), Some(&weak_new_behavior)),
            PolicyDecision::AutoExecute {
                action: ModerationAction::Ban,
                timeout_minutes: 1440,
            }
        );

        let harassment = verdict(ModerationCategory::Harassment, 0.95);
        assert_eq!(
            policy.decide_combined(Some(&harassment), Some(&weak_new_behavior)),
            PolicyDecision::Proposal {
                timeout_minutes: 1440,
            }
        );
    }
}
