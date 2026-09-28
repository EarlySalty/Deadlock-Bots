use serde_json::{json, Value};

use crate::action_policy::{ModerationAction, PolicyDecision};
use crate::behavior_detector::{BehaviorSignal, BehaviorTriggerType};
use crate::moderation_verdict::{ModerationCategory, ModerationVerdict};

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
    pub unresolved_consistency: bool,
    pub executed_actions: Vec<String>,
    pub mirrored_image_count: usize,
}

pub fn build_compact_case_embed(input: &CompactCaseEmbedInput) -> Value {
    let mut fields = vec![
        json!({
            "name": "Nutzer",
            "value": format!("<@{}> (`{}`)", input.user_id, truncate_chars(&input.user_tag, 80)),
            "inline": false,
        }),
        json!({
            "name": "Kanal",
            "value": format!("<#{}> · [zur Nachricht]({})", input.channel_id, case_jump_url(input.guild_id, input.channel_id, input.message_id)),
            "inline": false,
        }),
        json!({
            "name": "Kategorie",
            "value": effective_category_label(&input.verdict, input.behavior_signal.as_ref()),
            "inline": true,
        }),
        json!({
            "name": "Urteilssicherheit",
            "value": format!(
                "Analyse: {} ({:.0}%)\nPrüfung: {} ({:.0}%)\nDie Werte zeigen die Sicherheit des jeweiligen Urteils, nicht die Scam-Wahrscheinlichkeit.",
                category_label(&input.verdict.analysis.category),
                input.verdict.analysis.confidence * 100.0,
                verification_label(&input.verdict),
                input.verdict.verification.confidence * 100.0
            ),
            "inline": false,
        }),
        json!({
            "name": "Begründung",
            "value": truncate_chars(&input.verdict.verification.reason, DISCORD_FIELD_LIMIT),
            "inline": false,
        }),
        json!({
            "name": "Auslöser",
            "value": truncate_chars(&input.verdict.trigger, DISCORD_FIELD_LIMIT),
            "inline": false,
        }),
    ];
    if input.unresolved_consistency {
        fields.push(json!({
            "name": "Grund für manuelle Prüfung",
            "value": "Analyse und Prüfung widersprechen sich auch nach erneuter Prüfung. Keine automatische Sanktion.",
            "inline": false,
        }));
    }
    if let Some(signal) = &input.behavior_signal {
        fields.push(json!({
            "name": "Verhaltensmuster",
            "value": signal.trigger_label(),
            "inline": true,
        }));
        fields.push(json!({
            "name": "Aktivität",
            "value": format!(
                "{} Nachrichten · {} Kanäle · {}s",
                signal.evidence.message_count,
                signal.evidence.channel_ids.len(),
                signal.evidence.window_seconds
            ),
            "inline": true,
        }));
        fields.push(json!({
            "name": "Signale",
            "value": truncate_chars(&behavior_signal_summary(signal), DISCORD_FIELD_LIMIT),
            "inline": false,
        }));
        if !signal.evidence.image_urls.is_empty() {
            fields.push(json!({
                "name": "Bilder",
                "value": if input.mirrored_image_count > 0 {
                    format!("{} Bild(er) als Anhang gespiegelt", input.mirrored_image_count)
                } else {
                    "Bild(er) erkannt, Spiegelung fehlgeschlagen".to_string()
                },
                "inline": false,
            }));
        }
    }
    if !input.executed_actions.is_empty() {
        fields.push(json!({
            "name": "Ausgeführt",
            "value": truncate_chars(&input.executed_actions.join("\n"), DISCORD_FIELD_LIMIT),
            "inline": false,
        }));
    }

    json!({
        "title": match input.policy_decision {
            PolicyDecision::AutoExecute { .. } => "🚨 Automatisch vollzogen",
            PolicyDecision::Proposal { .. } => "⚠️ Bitte prüfen",
            PolicyDecision::Ignore => "ℹ️ Beobachtet",
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
            ModerationAction::DeleteOnly => Vec::new(),
            ModerationAction::Timeout => vec![json!({
                "type": 2,
                "style": 3,
                "label": "Timeout aufheben",
                "custom_id": format!("aimod:untimeout:{case_id}"),
            })],
            ModerationAction::Ban => vec![json!({
                "type": 2,
                "style": 3,
                "label": "Entbannen",
                "custom_id": format!("aimod:unban:{case_id}"),
            })],
        },
        PolicyDecision::Proposal { .. } => vec![
            json!({
                "type": 2,
                "style": 3,
                "label": "Übernehmen",
                "custom_id": format!("aimod:accept:{case_id}"),
            }),
            json!({
                "type": 2,
                "style": 4,
                "label": "Bannen",
                "custom_id": format!("aimod:ban:{case_id}"),
            }),
            json!({
                "type": 2,
                "style": 2,
                "label": "Verwerfen",
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

fn effective_category_label(
    verdict: &ModerationVerdict,
    behavior_signal: Option<&BehaviorSignal>,
) -> String {
    if let Some(signal) = behavior_signal {
        if verdict.trigger == signal.trigger_label()
            && verdict.verification.reason == signal.reason_code
        {
            let label = match signal.trigger_type {
                BehaviorTriggerType::AccountTakeover => "Mögliche Kontoübernahme",
                BehaviorTriggerType::BurstRate => "Viele Nachrichten in kurzer Zeit",
                BehaviorTriggerType::YoungAccountBurst => "Viele Nachrichten von neuem Konto",
                BehaviorTriggerType::ImageMultichannel => "Bilder in mehreren Kanälen",
                BehaviorTriggerType::ForeignInvite => "Fremde Einladung",
                BehaviorTriggerType::Keyword => "Auffälliges Stichwort",
            };
            return format!("Verhaltensmuster: {label}");
        }
    }
    let label = category_label(&verdict.verification.category);
    if verdict.verification.confirmed {
        label.to_string()
    } else {
        format!("{label} (nicht bestätigt)")
    }
}

fn category_label(category: &ModerationCategory) -> &'static str {
    match category {
        ModerationCategory::Scam => "Betrugsversuch",
        ModerationCategory::Csam => "Sexualisierte Inhalte mit Minderjährigen",
        ModerationCategory::NsfwExplicit => "Expliziter sexueller Inhalt",
        ModerationCategory::Harassment => "Belästigung",
        ModerationCategory::HateSpeech => "Hassrede",
        ModerationCategory::RagebaitOk => "Provokation ohne Regelverstoß",
        ModerationCategory::GameRelatedOk => "Spielbezogen, unauffällig",
        ModerationCategory::Other => "Sonstiges",
    }
}

fn verification_label(verdict: &ModerationVerdict) -> String {
    let label = category_label(&verdict.verification.category);
    if verdict.verification.confirmed {
        format!("Bestätigt: {label}")
    } else {
        format!("Nicht bestätigt: {label}")
    }
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
            unresolved_consistency: false,
            executed_actions: vec!["delete:ok".to_string(), "timeout:ok".to_string()],
            mirrored_image_count: 0,
        });
        let serialized = embed.to_string();

        assert!(!serialized.contains("Case-ID"));
        assert!(serialized.contains("Kategorie"));
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
        assert!(serialized.contains("Übernehmen"));
    }

    #[test]
    fn unconfirmed_verdict_and_unresolved_conflict_are_explicit() {
        let verdict = ModerationVerdict {
            analysis: ContentAnalysis {
                category: ModerationCategory::Other,
                confidence: 0.86,
                reason: "Analysegrund".to_string(),
                raw_json: "{}".to_string(),
            },
            verification: VerificationDecision {
                confirmed: false,
                category: ModerationCategory::Scam,
                confidence: 0.92,
                reason: "Prüfgrund".to_string(),
                raw_json: "{}".to_string(),
            },
            trigger: "Bild".to_string(),
        };
        let embed = build_compact_case_embed(&CompactCaseEmbedInput {
            guild_id: 1,
            channel_id: 2,
            message_id: 3,
            user_id: 4,
            user_tag: "Anna".to_string(),
            verdict,
            behavior_signal: None,
            policy_decision: PolicyDecision::Proposal {
                timeout_minutes: 60,
            },
            unresolved_consistency: true,
            executed_actions: Vec::new(),
            mirrored_image_count: 0,
        });
        let fields = embed["fields"].as_array().expect("fields");
        assert_eq!(fields[2]["value"], "Betrugsversuch (nicht bestätigt)");
        let certainty = fields[3]["value"].as_str().expect("certainty");
        assert!(certainty.contains("Analyse: Sonstiges (86%)"));
        assert!(certainty.contains("Prüfung: Nicht bestätigt: Betrugsversuch (92%)"));
        assert!(certainty.contains("nicht die Scam-Wahrscheinlichkeit"));
        assert_eq!(fields[6]["name"], "Grund für manuelle Prüfung");
        assert!(fields[6]["value"]
            .as_str()
            .expect("review reason")
            .contains("widersprechen"));
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
            unresolved_consistency: false,
            executed_actions: Vec::new(),
            mirrored_image_count: 1,
        });
        let serialized = embed.to_string();

        assert!(!serialized.contains("Case-ID"));
        assert!(serialized.contains("account_takeover"));
        assert!(serialized.contains("2 Nachrichten · 2 Kanäle · 30s"));
        assert!(serialized.contains("1 Bild"));
        assert!(!serialized.contains("https://img/1.png"));

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
