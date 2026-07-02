use serde_json::{json, Value};

use crate::action_policy::PolicyDecision;
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
        PolicyDecision::AutoExecute { .. } => vec![json!({
            "type": 2,
            "style": 3,
            "label": "PLATZHALTER: Timeout-aufheben-Button",
            "custom_id": format!("aimod:untimeout:{case_id}"),
        })],
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
    use crate::action_policy::PolicyDecision;
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
            policy_decision: PolicyDecision::AutoExecute {
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
}
