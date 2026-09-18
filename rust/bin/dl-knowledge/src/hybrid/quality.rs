use std::collections::HashSet;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{Candidate, Chunk, LlmSelection};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub question: String,
    pub answerable: bool,
    pub expected_sources: Vec<String>,
    pub context_terms: Vec<String>,
    pub answer_terms: Vec<String>,
    pub forbidden_terms: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Quality {
    pub ranked_paths: Vec<String>,
    pub hit: bool,
    pub source_recall: Option<f64>,
    pub reciprocal_rank: Option<f64>,
    pub candidates: usize,
    pub rejected_by_relevance: usize,
    pub relevant_candidates: usize,
    pub expected_candidates_before: usize,
    pub expected_candidates_after: usize,
    pub expected_source_lost_to_relevance: bool,
    pub answer_terms_lost_to_relevance: bool,
    pub missing_context_terms: Vec<String>,
    pub forbidden_context_terms: Vec<String>,
    pub grounded_selection_possible: Option<bool>,
}

pub fn measure(case: &Case, chunks: &[Chunk]) -> Quality {
    let candidates = crate::candidates_for(chunks);
    let relevant = candidates
        .iter()
        .filter(|candidate| crate::candidate_is_relevant(&case.question, candidate))
        .collect::<Vec<_>>();
    let expected = case
        .expected_sources
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let found = chunks
        .iter()
        .map(|chunk| chunk.path.as_str())
        .collect::<HashSet<_>>();
    let first = chunks
        .iter()
        .position(|chunk| expected.contains(chunk.path.as_str()));
    let context = crate::build_prompt("", &candidates).to_lowercase();
    let before = candidates
        .iter()
        .filter(|candidate| expected.contains(candidate.path.as_str()))
        .collect::<Vec<_>>();
    let after = relevant
        .iter()
        .copied()
        .filter(|candidate| expected.contains(candidate.path.as_str()))
        .collect::<Vec<_>>();
    let before_text = before
        .iter()
        .map(|candidate| candidate.passage.rendered())
        .collect::<Vec<_>>()
        .join("\n\n")
        .to_lowercase();
    let after_text = after
        .iter()
        .map(|candidate| candidate.passage.rendered())
        .collect::<Vec<_>>()
        .join("\n\n")
        .to_lowercase();
    Quality {
        ranked_paths: chunks.iter().map(|chunk| chunk.path.clone()).collect(),
        hit: first.is_some(),
        source_recall: case
            .answerable
            .then(|| expected.intersection(&found).count() as f64 / expected.len() as f64),
        reciprocal_rank: case
            .answerable
            .then(|| first.map_or(0.0, |rank| 1.0 / (rank + 1) as f64)),
        candidates: candidates.len(),
        rejected_by_relevance: candidates.len() - relevant.len(),
        relevant_candidates: relevant.len(),
        expected_candidates_before: before.len(),
        expected_candidates_after: after.len(),
        expected_source_lost_to_relevance: !before.is_empty() && after.is_empty(),
        answer_terms_lost_to_relevance: case.answerable
            && contains_all(&before_text, &case.answer_terms)
            && !contains_all(&after_text, &case.answer_terms),
        missing_context_terms: case
            .context_terms
            .iter()
            .filter(|term| !context.contains(&term.to_lowercase()))
            .cloned()
            .collect(),
        forbidden_context_terms: case
            .forbidden_terms
            .iter()
            .filter(|term| context.contains(&term.to_lowercase()))
            .cloned()
            .collect(),
        grounded_selection_possible: case
            .answerable
            .then(|| grounded_selection(case, &candidates, &after)),
    }
}

fn contains_all(text: &str, terms: &[String]) -> bool {
    terms.iter().all(|term| text.contains(&term.to_lowercase()))
}

fn grounded_selection(case: &Case, candidates: &[Candidate], relevant: &[&Candidate]) -> bool {
    if relevant.is_empty() {
        return false;
    }
    if case.answer_terms.iter().all(|term| !term.contains('\n')) {
        let union = relevant
            .iter()
            .map(|candidate| candidate.passage.rendered())
            .collect::<Vec<_>>()
            .join("\n\n")
            .to_lowercase();
        if !contains_all(&union, &case.answer_terms) {
            return false;
        }
    }
    let mut subsets = vec![(0usize, Vec::<String>::new())];
    while let Some((start, selected_ids)) = subsets.pop() {
        for (index, candidate) in relevant.iter().enumerate().skip(start) {
            let mut ids = selected_ids.clone();
            ids.push(candidate.id.clone());
            let selection = LlmSelection { candidate_ids: ids };
            if crate::grounded_response(&case.question, &selection, candidates)
                .and_then(|response| response.answer)
                .is_some_and(|answer| contains_all(&answer.to_lowercase(), &case.answer_terms))
            {
                return true;
            }
            if selection.candidate_ids.len() < crate::MAX_SELECTED_CANDIDATES {
                subsets.push((index + 1, selection.candidate_ids));
            }
        }
    }
    false
}

pub fn load(dir: &Path, root: &Path) -> Result<Vec<(String, usize, Case)>> {
    let names = [
        "public-discord-core.json",
        "public-discord-tools.json",
        "public-integration.json",
        "public-patchnotes-turniere.json",
        "public-steam-website.json",
        "public-twitch.json",
    ];
    let mut questions = HashSet::new();
    let mut result = Vec::new();
    for name in names {
        let raw = std::fs::read(dir.join(name)).context("Golden-Datei fehlt")?;
        let cases: Vec<Case> = serde_json::from_slice(&raw)?;
        ensure!(!cases.is_empty(), "Leere Golden-Datei");
        for (row, case) in cases.into_iter().enumerate() {
            ensure!(
                !case.question.trim().is_empty()
                    && questions.insert(case.question.trim().to_string()),
                "Leere oder doppelte Golden-Frage"
            );
            ensure!(
                if case.answerable {
                    !case.expected_sources.is_empty()
                        && !case.context_terms.is_empty()
                        && !case.answer_terms.is_empty()
                } else {
                    case.expected_sources.is_empty()
                        && case.context_terms.is_empty()
                        && case.answer_terms.is_empty()
                },
                "Ungültige Golden-Labels"
            );
            for source in &case.expected_sources {
                let path = Path::new(source);
                ensure!(path.extension().is_some_and(|ext| ext == "html") && !source.contains('\\')
                    && path.components().all(|part| matches!(part, std::path::Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case("internal")))
                    && root.join(path).is_file(), "Ungültige oder fehlende Golden-Quelle");
            }
            result.push((name.to_string(), row + 1, case));
        }
    }
    ensure!(
        result.len() == 224,
        "Messung benötigt die unveränderte 224er-Golden-Suite"
    );
    Ok(result)
}
