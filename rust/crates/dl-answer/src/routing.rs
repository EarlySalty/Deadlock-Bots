use std::collections::HashSet;

use dl_ai::{KnowledgeDecision, KnowledgeRouteRequest};
use serde::{Deserialize, Serialize};

use crate::{Answer, Evidence, Source};

pub(super) fn restricted_answer(decision: &KnowledgeDecision) -> Option<Answer> {
    (decision.restricted.is_finite() && (0.95..=1.0).contains(&decision.restricted)).then(|| Answer::Restricted { text: "Interne Erkennungsregeln und technische Schutzdetails bleiben intern. Bei öffentlich sichtbaren Berechtigungen, erlaubten Bot-Aktionen und dem Trennen einer Verbindung helfe ich dir gern. So kannst du prüfen, was du dem Bot für deinen Kanal erlaubst.".into() })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardAnswer {
    pub id: String,
    pub source: Source,
    pub source_sha256: String,
    pub question: String,
    pub scope: String,
    pub answer: String,
}

impl StandardAnswer {
    pub fn valid(&self) -> bool {
        let Source::CommunityPage { path, .. } = &self.source else {
            return false;
        };
        let prefix = format!("faq:{path}#");
        self.id
            .strip_prefix(&prefix)
            .is_some_and(|section| !section.is_empty() && !section.chars().any(char::is_whitespace))
            && self.source.valid()
            && self.source_sha256.len() == 64
            && self.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            && !self.question.trim().is_empty()
            && self.question.encode_utf16().count() <= 500
            && !self.scope.trim().is_empty()
            && self.scope.encode_utf16().count() <= 800
            && !self.answer.trim().is_empty()
            && self.answer.encode_utf16().count() <= 1600
            && !self
                .answer
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    }
}

pub(super) fn eligible_standards(
    items: Vec<StandardAnswer>,
    standalone: bool,
) -> Vec<StandardAnswer> {
    if !standalone {
        return Vec::new();
    }
    let mut ids = HashSet::new();
    items
        .into_iter()
        .filter(|item| item.valid() && ids.insert(item.id.clone()))
        .take(32)
        .collect()
}

pub(super) fn request(
    question: &str,
    context: &str,
    evidence: &[Evidence],
    standards: &[StandardAnswer],
) -> KnowledgeRouteRequest {
    KnowledgeRouteRequest {
        question: question.into(),
        conversation_context: context.into(),
        evidence: evidence
            .iter()
            .map(|item| serde_json::json!(item))
            .collect(),
        standard_answers: standards
            .iter()
            .map(|item| serde_json::json!(item))
            .collect(),
    }
}

pub(super) fn direct_answer(
    decision: &KnowledgeDecision,
    standards: &[StandardAnswer],
) -> Option<Answer> {
    let (id, confidence) = decision.standard.as_ref()?;
    if !confidence.is_finite() || *confidence < 0.99 || *confidence > 1.0 {
        return None;
    }
    let candidate = standards
        .iter()
        .find(|item| &item.id == id && item.valid())?;
    Some(Answer::Grounded {
        text: candidate.answer.clone(),
        sources: vec![candidate.source.clone()],
        intent: None,
        pate_request: false,
        unavailable_sources: Vec::new(),
    })
}

/// A valid categorical decision is authoritative for relevance, never for facts.
/// Only transport/schema failures use the unchanged original retrieval fallback.
pub(super) fn select_evidence(
    decision: &KnowledgeDecision,
    evidence: Vec<Evidence>,
) -> Vec<Evidence> {
    if decision.relevance.len() != evidence.len()
        || evidence
            .iter()
            .any(|item| !decision.relevance.contains_key(&item.id))
    {
        return without_optional_code(evidence);
    }
    evidence
        .into_iter()
        .filter(|item| decision.relevance[&item.id].supports_question())
        .collect()
}

pub(super) fn clarification_answer() -> Answer {
    Answer::Clarification { text: "Worauf beziehst du dich genau? Sag mir bitte, um welche Funktion oder Verbindung es geht und was du schon ausprobiert hast.".into() }
}

pub(super) fn without_optional_code(evidence: Vec<Evidence>) -> Vec<Evidence> {
    evidence
        .into_iter()
        .filter(|item| !matches!(item.source, Source::PublicCodeEvidence { .. }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricted_decision_has_a_fixed_general_reply_but_uncertain_is_not_blocked() {
        let mut decision = KnowledgeDecision {
            restricted: 1.0,
            ..Default::default()
        };
        assert!(matches!(
            restricted_answer(&decision),
            Some(Answer::Restricted { .. })
        ));
        decision.restricted = 0.5;
        assert!(restricted_answer(&decision).is_none());
    }
    fn evidence(id: &str) -> Evidence {
        Evidence {
            id: id.into(),
            source: if id.starts_with('C') {
                Source::CommunityPage {
                    title: "Hilfe".into(),
                    path: "hilfe.html".into(),
                }
            } else {
                Source::GameData {
                    title: "Spiel".into(),
                }
            },
            text: "Beleg".into(),
            observed_at: None,
        }
    }
    fn standard() -> StandardAnswer {
        StandardAnswer {
            id: "faq:hilfe.html#start".into(),
            source: evidence("C1").source,
            source_sha256: "a".repeat(64),
            question: "Wie geht das?".into(),
            scope: "Allgemeine Einrichtung".into(),
            answer: "So geht es.".into(),
        }
    }
    #[test]
    fn uncertain_or_invalid_standard_never_answers_directly() {
        let standards = vec![standard()];
        for (id, score) in [("faq:hilfe.html#start", 0.98), ("unknown", 1.0)] {
            let decision = KnowledgeDecision {
                standard: Some((id.into(), score)),
                ..Default::default()
            };
            assert!(direct_answer(&decision, &standards).is_none());
        }
        assert!(eligible_standards(standards.clone(), false).is_empty());
        let mut invalid = standards[0].clone();
        invalid.source_sha256.clear();
        assert!(eligible_standards(vec![invalid], true).is_empty());
    }
    #[test]
    fn direct_answer_is_verbatim_and_never_claims_an_action() {
        let decision = KnowledgeDecision {
            standard: Some((standard().id, 1.0)),
            ..Default::default()
        };
        let Answer::Grounded {
            text,
            intent,
            pate_request,
            ..
        } = direct_answer(&decision, &[standard()]).expect("direct")
        else {
            panic!("grounded")
        };
        assert_eq!(text, "So geht es.");
        assert!(intent.is_none());
        assert!(!pate_request);
    }
    #[test]
    fn categorical_roles_preserve_real_mixed_support_without_unrelated_fallback() {
        use dl_ai::EvidenceRole::*;
        let decision = KnowledgeDecision {
            relevance: [
                ("C1".into(), Partial),
                ("C2".into(), Related),
                ("G1".into(), Full),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let selected = select_evidence(
            &decision,
            vec![evidence("C1"), evidence("C2"), evidence("G1")],
        );
        assert_eq!(
            selected.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["C1", "G1"]
        );
        let empty = KnowledgeDecision {
            relevance: [("C1".into(), Related)].into_iter().collect(),
            ..Default::default()
        };
        assert!(select_evidence(&empty, vec![evidence("C1")]).is_empty());
        assert_eq!(
            select_evidence(&KnowledgeDecision::default(), vec![evidence("C1")]).len(),
            1
        );
    }
}
