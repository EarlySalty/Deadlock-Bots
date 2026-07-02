use crate::moderation_verdict::ModerationVerdict;

#[derive(Debug, Clone, PartialEq)]
pub struct ActionPolicyConfig {
    pub auto_execute_verified_confidence: f64,
    pub proposal_verified_confidence: f64,
    pub timeout_minutes: i64,
}

impl Default for ActionPolicyConfig {
    fn default() -> Self {
        Self {
            auto_execute_verified_confidence: 0.85,
            proposal_verified_confidence: 0.60,
            timeout_minutes: 1440,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Ignore,
    AutoExecute { timeout_minutes: i64 },
    Proposal { timeout_minutes: i64 },
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
        if !verdict.verification.confirmed
            || verdict.verification.confidence < self.config.proposal_verified_confidence
        {
            return PolicyDecision::Ignore;
        }

        if verdict.verification.category.is_high_damage()
            && verdict.verification.confidence >= self.config.auto_execute_verified_confidence
        {
            return PolicyDecision::AutoExecute {
                timeout_minutes: self.config.timeout_minutes,
            };
        }

        PolicyDecision::Proposal {
            timeout_minutes: self.config.timeout_minutes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                timeout_minutes: 1440
            }
        );
        assert_eq!(
            policy.decide(&verdict(ModerationCategory::Csam, 0.85)),
            PolicyDecision::AutoExecute {
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
}
