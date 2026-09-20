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
    pub evidence_id: String,
    pub question: String,
    pub scope: String,
    pub answer: String,
}

impl StandardAnswer {
    pub fn valid(&self, evidence: &[Evidence]) -> bool {
        let Some(item) = evidence.iter().find(|item| item.id == self.evidence_id) else {
            return false;
        };
        let Source::CommunityPage { path, .. } = &item.source else {
            return false;
        };
        let prefix = format!("faq:{path}#");
        self.id
            .strip_prefix(&prefix)
            .is_some_and(|section| !section.is_empty() && !section.chars().any(char::is_whitespace))
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
    evidence: &[Evidence],
    standalone: bool,
) -> Vec<StandardAnswer> {
    if !standalone {
        return Vec::new();
    }
    let mut ids = HashSet::new();
    items
        .into_iter()
        .filter(|item| item.valid(evidence) && ids.insert(item.id.clone()))
        .take(12)
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
    evidence: &[Evidence],
) -> Option<Answer> {
    let (id, confidence) = decision.standard.as_ref()?;
    if !confidence.is_finite() || *confidence < 0.99 || *confidence > 1.0 {
        return None;
    }
    let candidate = standards
        .iter()
        .find(|item| &item.id == id && item.valid(evidence))?;
    let source = evidence
        .iter()
        .find(|item| item.id == candidate.evidence_id)?
        .source
        .clone();
    Some(Answer::Grounded {
        text: candidate.answer.clone(),
        sources: vec![source],
        intent: None,
        pate_request: false,
        unavailable_sources: Vec::new(),
    })
}

/// Uncertain passages remain available. Only clearly irrelevant passages may
/// be removed, and never a whole retrieved domain from a mixed question.
pub(super) fn select_evidence(
    decision: &KnowledgeDecision,
    evidence: Vec<Evidence>,
) -> Vec<Evidence> {
    if decision.relevance.len() != evidence.len()
        || evidence.iter().any(|item| {
            decision
                .relevance
                .get(&item.id)
                .is_none_or(|score| !score.is_finite() || !(0.0..=1.0).contains(score))
        })
    {
        return without_optional_code(evidence);
    }
    let evidence = evidence
        .into_iter()
        .filter(|item| {
            !matches!(item.source, Source::PublicCodeEvidence { .. })
                || decision.relevance[&item.id] >= 0.95
        })
        .collect::<Vec<_>>();
    if !decision.relevance.values().any(|score| *score >= 0.95) {
        return evidence;
    }
    let selected = evidence
        .iter()
        .filter(|item| decision.relevance[&item.id] > 0.05)
        .cloned()
        .collect::<Vec<_>>();
    let domain = |item: &Evidence| {
        matches!(
            item.source,
            Source::CommunityPage { .. } | Source::PublicCodeEvidence { .. }
        )
    };
    if selected.is_empty()
        || evidence
            .iter()
            .any(|item| !selected.iter().any(|chosen| domain(item) == domain(chosen)))
    {
        return evidence;
    }
    selected
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
    fn optional_code_requires_positive_selection_even_on_fallback() {
        let mut code = evidence("P1");
        code.source = Source::PublicCodeEvidence {
            title: "FAQ".into(),
            path: "faq.html".into(),
            repository: "discord".into(),
            release_commit: "a".repeat(40),
            source_path: "rust/crates/dl-community/src/faq.rs".into(),
            symbol: "register:faq".into(),
            blob_sha256: "b".repeat(64),
        };
        for score in [0.0, 0.5, 0.94, 0.95, 1.0] {
            let items = vec![evidence("C1"), evidence("G1"), code.clone()];
            let decision = KnowledgeDecision {
                relevance: [("C1".into(), 0.5), ("G1".into(), 0.5), ("P1".into(), score)]
                    .into_iter()
                    .collect(),
                ..Default::default()
            };
            assert_eq!(
                select_evidence(&decision, items.clone()).len(),
                if score >= 0.95 { 3 } else { 2 }
            );
            assert_eq!(
                select_evidence(&KnowledgeDecision::default(), items.clone()).len(),
                2
            );
            assert_eq!(without_optional_code(items).len(), 2);
        }
    }
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
            evidence_id: "C1".into(),
            question: "Wie geht das?".into(),
            scope: "Allgemeine Einrichtung".into(),
            answer: "So geht es.".into(),
        }
    }
    #[test]
    fn uncertain_or_invalid_standard_never_answers_directly() {
        let items = vec![evidence("C1")];
        let standards = vec![standard()];
        for (id, score) in [("faq:hilfe.html#start", 0.98), ("unknown", 1.0)] {
            let decision = KnowledgeDecision {
                standard: Some((id.into(), score)),
                ..Default::default()
            };
            assert!(direct_answer(&decision, &standards, &items).is_none());
        }
        assert!(eligible_standards(standards.clone(), &items, false).is_empty());
        assert!(eligible_standards(standards, &[evidence("C2")], true).is_empty());
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
        } = direct_answer(&decision, &[standard()], &[evidence("C1")]).expect("direct")
        else {
            panic!("grounded")
        };
        assert_eq!(text, "So geht es.");
        assert!(intent.is_none());
        assert!(!pate_request);
    }
    #[test]
    fn mixed_question_and_uncertain_decisions_keep_existing_evidence() {
        for scores in [[("C1", 1.0), ("G1", 0.0)], [("C1", 0.7), ("G1", 1.0)]] {
            let decision = KnowledgeDecision {
                relevance: scores.into_iter().map(|(id, s)| (id.into(), s)).collect(),
                ..Default::default()
            };
            assert_eq!(
                select_evidence(&decision, vec![evidence("C1"), evidence("G1")]).len(),
                2
            );
        }
    }
    #[test]
    fn confident_irrelevant_passage_can_be_removed_with_domain_preserved() {
        let decision = KnowledgeDecision {
            relevance: [("C1".into(), 1.0), ("C2".into(), 0.0)]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let selected = select_evidence(&decision, vec![evidence("C1"), evidence("C2")]);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "C1");
    }
}
