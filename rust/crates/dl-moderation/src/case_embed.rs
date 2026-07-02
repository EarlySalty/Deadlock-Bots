use serde_json::{json, Value};

use crate::action_policy::{ModerationAction, PolicyDecision};
use crate::behavior_detector::BehaviorSignal;
use crate::moderation_verdict::ModerationVerdict;

const DISCORD_FIELD_LIMIT: usize = 1024;

#[derive(Debug, Clone)]
pub struct CompactCaseEmbedInput {
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub user_id: u64,
    pub user_tag: String,
    pub verdict: ModerationVerdict,
    pub behavior_signal: Option<BehaviorSignal>,
    pub policy_decision: PolicyDecision,
    pub executed_actions: Vec<String>,
}

pub fn build_compact_case_embed(input: &CompactCaseEmbedInput) -> Value {
    let mut fields = vec![
        json!({
            "name": "PLATZHALTER: User-Feld",
            "value": format!("<@{}> (`{}`)", input.user_id, truncate_chars(&input.user_tag, 80)),
            "inline": false,
        }),
        json!({
            "name": "PLATZHALTER: Kanal-Feld",
            "value": format!("<#{}> | [Jump]({})", input.channel_id, case_jump_url(input.guild_id, input.channel_id, input.message_id)),
            "inline": false,
        }),
        json!({
            "name": "PLATZHALTER: Kategorie-Feld",
            "value": input.verdict.verification.category.as_label(),
            "inline": true,
        }),
        json!({
            "name": "PLATZHALTER: Confidence-Feld",
            "value": format!(
                "Analyze {:.0}% | Verify {:.0}%",
                input.verdict.analysis.confidence * 100.0,
                input.verdict.verification.confidence * 100.0
            ),
            "inline": true,
        }),
        json!({
            "name": "PLATZHALTER: Begruendung-Feld",
            "value": truncate_chars(&input.verdict.verification.reason, DISCORD_FIELD_LIMIT),
            "inline": false,
        }),
        json!({
            "name": "PLATZHALTER: Ausloeser-Feld",
            "value": truncate_chars(&input.verdict.trigger, DISCORD_FIELD_LIMIT),
            "inline": false,
        }),
    ];
    if let Some(signal) = &input.behavior_signal {
        fields.push(json!({
            "name": "PLATZHALTER: Verhalten-Trigger-Feld",
            "value": signal.trigger_label(),
            "inline": true,
        }));
        fields.push(json!({
            "name": "PLATZHALTER: Aktivitaetsfenster-Feld",
            "value": format!(
                "{} messages / {} channels / {}s",
                signal.evidence.message_count,
                signal.evidence.channel_ids.len(),
                signal.evidence.window_seconds
            ),
            "inline": true,
        }));
        fields.push(json!({
            "name": "PLATZHALTER: Verhalten-Signale-Feld",
            "value": truncate_chars(&behavior_signal_summary(signal), DISCORD_FIELD_LIMIT),
            "inline": false,
        }));
        if !signal.evidence.image_urls.is_empty() {
            fields.push(json!({
                "name": "PLATZHALTER: Beweisbilder-Feld",
                "value": truncate_chars(&signal.evidence.image_urls.join("\n"), DISCORD_FIELD_LIMIT),
                "inline": false,
            }));
        }
    }
    if !input.executed_actions.is_empty() {
        fields.push(json!({
            "name": "PLATZHALTER: Aktionen-Feld",
            "value": truncate_chars(&input.executed_actions.join("\n"), DISCORD_FIELD_LIMIT),
            "inline": false,
        }));
    }

    json!({
        "title": match input.policy_decision {
            PolicyDecision::AutoExecute { .. } => "PLATZHALTER: Auto-Vollzug-Embed-Titel",
            PolicyDecision::Proposal { .. } => "PLATZHALTER: Vorschlag-Embed-Titel",
            PolicyDecision::Ignore => "PLATZHALTER: Ignoriert-Embed-Titel",
        },
        "color": match input.policy_decision {
            PolicyDecision::AutoExecute { .. } => 0xED4245,
            PolicyDecision::Proposal { .. } => 0xFEE75C,
            PolicyDecision::Ignore => 0x5865F2,
        },
        "fields": fields,
    })
}

pub fn build_case_components(case_id: &str, policy_decision: &PolicyDecision) -> Value {
    let buttons = match policy_decision {
        PolicyDecision::AutoExecute { action, .. } => match action {
            ModerationAction::Timeout => vec![json!({
                "type": 2,
                "style": 3,
                "label": "PLATZHALTER: Timeout-aufheben-Button",
                "custom_id": format!("aimod:untimeout:{case_id}"),
            })],
            ModerationAction::Ban => vec![json!({
                "type": 2,
                "style": 3,
                "label": "PLATZHALTER: Entbannen-Button",
                "custom_id": format!("aimod:unban:{case_id}"),
            })],
        },
        PolicyDecision::Proposal { .. } => vec![
            json!({
                "type": 2,
                "style": 3,
                "label": "PLATZHALTER: Annehmen-Button",
                "custom_id": format!("aimod:accept:{case_id}"),
            }),
            json!({
                "type": 2,
                "style": 4,
                "label": "PLATZHALTER: Ban-Button",
                "custom_id": format!("aimod:ban:{case_id}"),
            }),
            json!({
                "type": 2,
                "style": 2,
                "label": "PLATZHALTER: Ablehnen-Button",
                "custom_id": format!("aimod:deny:{case_id}"),
            }),
        ],
        PolicyDecision::Ignore => Vec::new(),
    };
    json!([{ "type": 1, "components": buttons }])
}

fn behavior_signal_summary(signal: &BehaviorSignal) -> String {
    let join_age = signal
        .evidence
        .join_age_hours
        .map(|hours| hours.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let invite = signal.evidence.invite_code.as_deref().unwrap_or("n/a");
    format!(
        "severity={} action_hint={} images={} attachments={} keyword={} account_age_h={} join_age_h={} invite={}",
        signal.severity.as_label(),
        signal.action_hint.as_label(),
        signal.evidence.image_count,
        signal.evidence.attachment_count,
        signal.evidence.keyword_hit,
        signal.evidence.account_age_hours,
        join_age,
        invite
    )
}

fn case_jump_url(guild_id: u64, channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_policy::{ModerationAction, PolicyDecision};
    use crate::behavior_detector::{
        BehaviorActionHint, BehaviorEvidence, BehaviorSeverity, BehaviorSignal, BehaviorTriggerType,
    };
    use crate::moderation_verdict::{
        ContentAnalysis, ModerationCategory, ModerationVerdict, VerificationDecision,
    };

    #[test]
    fn compact_embed_has_no_visible_case_id_and_contains_core_fields() {
        let verdict = ModerationVerdict {
            analysis: ContentAnalysis {
                category: ModerationCategory::Scam,
                confidence: 0.66,
                reason: "Analysegrund".to_string(),
                raw_json: "{}".to_string(),
            },
            verification: VerificationDecision {
                confirmed: true,
                category: ModerationCategory::Scam,
                confidence: 0.88,
                reason: "Verifiergrund".to_string(),
                raw_json: "{}".to_string(),
            },
            trigger: "free crypto".to_string(),
        };
        let embed = build_compact_case_embed(&CompactCaseEmbedInput {
            guild_id: 1,
            channel_id: 2,
            message_id: 3,
            user_id: 4,
            user_tag: "Anna".to_string(),
            verdict,
            behavior_signal: None,
            policy_decision: PolicyDecision::AutoExecute {
                action: ModerationAction::Timeout,
                timeout_minutes: 1440,
            },
            executed_actions: vec!["delete:ok".to_string(), "timeout:ok".to_string()],
        });
        let serialized = embed.to_string();

        assert!(!serialized.contains("Case-ID"));
        assert!(serialized.contains("PLATZHALTER"));
        assert!(serialized.contains("66%"));
        assert!(serialized.contains("88%"));
        assert!(serialized.contains("free crypto"));
    }

    #[test]
    fn components_keep_case_id_only_in_button_payload() {
        let components = build_case_components(
            "case-123",
            &PolicyDecision::Proposal {
                timeout_minutes: 1440,
            },
        );
        let serialized = components.to_string();

        assert!(serialized.contains("aimod:accept:case-123"));
        assert!(serialized.contains("PLATZHALTER: Annehmen-Button"));
    }

    #[test]
    fn compact_embed_renders_behavior_fields_without_visible_case_id() {
        let verdict = ModerationVerdict {
            analysis: ContentAnalysis {
                category: ModerationCategory::Other,
                confidence: 1.0,
                reason: "behavior:account_takeover".to_string(),
                raw_json: "{}".to_string(),
            },
            verification: VerificationDecision {
                confirmed: true,
                category: ModerationCategory::Other,
                confidence: 1.0,
                reason: "behavior:account_takeover".to_string(),
                raw_json: "{}".to_string(),
            },
            trigger: "account_takeover".to_string(),
        };
        let signal = BehaviorSignal {
            trigger_type: BehaviorTriggerType::AccountTakeover,
            severity: BehaviorSeverity::Critical,
            action_hint: BehaviorActionHint::Ban,
            reason_code: "behavior:account_takeover".to_string(),
            account_is_new: true,
            evidence: BehaviorEvidence {
                window_seconds: 30,
                channel_ids: vec![10, 11],
                message_ids: vec![100, 101],
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
        };

        let embed = build_compact_case_embed(&CompactCaseEmbedInput {
            guild_id: 1,
            channel_id: 10,
            message_id: 100,
            user_id: 4,
            user_tag: "Anna".to_string(),
            verdict,
            behavior_signal: Some(signal),
            policy_decision: PolicyDecision::AutoExecute {
                action: ModerationAction::Ban,
                timeout_minutes: 1440,
            },
            executed_actions: Vec::new(),
        });
        let serialized = embed.to_string();

        assert!(!serialized.contains("Case-ID"));
        assert!(serialized.contains("account_takeover"));
        assert!(serialized.contains("2 messages / 2 channels / 30s"));
        assert!(serialized.contains("https://img/1.png"));

        let components = build_case_components(
            "case-123",
            &PolicyDecision::AutoExecute {
                action: ModerationAction::Ban,
                timeout_minutes: 1440,
            },
        );
        assert!(components.to_string().contains("aimod:unban:case-123"));
    }
}
