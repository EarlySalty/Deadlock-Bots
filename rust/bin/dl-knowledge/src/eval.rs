use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use super::{candidate_is_relevant, candidates_for, load_corpus, Chunk, KnowledgeBase};

const GOLDEN_FILES: [&str; 6] = [
    "public-discord-core.json",
    "public-discord-tools.json",
    "public-integration.json",
    "public-patchnotes-turniere.json",
    "public-steam-website.json",
    "public-twitch.json",
];
const BASELINE_GOLDEN_CASES: usize = 224;
const RETRIEVAL_LIMIT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum EvalMode {
    Bm25,
    Dense,
    Hybrid,
    HybridReranker,
}

impl EvalMode {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "bm25" => Ok(Self::Bm25),
            "dense" => Ok(Self::Dense),
            "hybrid" => Ok(Self::Hybrid),
            "hybrid-reranker" | "hybrid+reranker" => Ok(Self::HybridReranker),
            _ => bail!("Unbekannter Eval-Modus: {raw}"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Bm25 => "BM25",
            Self::Dense => "Dense",
            Self::Hybrid => "Hybrid",
            Self::HybridReranker => "Hybrid + Reranker",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenCase {
    question: String,
    answerable: bool,
    expected_sources: Vec<String>,
    context_terms: Vec<String>,
    answer_terms: Vec<String>,
    forbidden_terms: Vec<String>,
    #[serde(default)]
    herkunft: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CaseResult {
    question: String,
    answerable: bool,
    expected_sources: Vec<String>,
    ranked_paths: Vec<String>,
    first_relevant_rank: Option<usize>,
    retrieval_answerable: bool,
}

#[derive(Debug, Clone, Serialize)]
struct Metrics {
    answerable_cases: usize,
    unanswerable_cases: usize,
    recall_at_1: f64,
    recall_at_3: f64,
    recall_at_5: f64,
    mrr: f64,
    citation_correctness: f64,
    abstain_rate: f64,
    false_answer_rate: f64,
}

#[derive(Debug, Serialize)]
struct EvalReport {
    mode: EvalMode,
    generated_unix_seconds: u64,
    golden_cases: usize,
    synthetic_cases: usize,
    retrieval_limit: usize,
    metrics: Metrics,
    cases: Vec<CaseResult>,
}

#[derive(Debug, Deserialize)]
struct HybridBenchReport {
    case_results: Vec<HybridBenchCase>,
}

#[derive(Debug, Deserialize)]
struct HybridBenchCase {
    question: String,
    answerable: bool,
    expected_sources: Vec<String>,
    dense_only: HybridBenchRetrieval,
    hybrid: HybridBenchRetrieval,
    reranked: HybridBenchRetrieval,
}

#[derive(Debug, Deserialize)]
struct HybridBenchRetrieval {
    ranked_paths: Vec<String>,
    relevant_candidates: usize,
}

pub fn run() -> Result<bool> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("eval") {
        return Ok(false);
    }

    if args.get(1).is_none()
        || matches!(args.get(1).map(String::as_str), Some("help" | "--help"))
    {
        print_help();
        return Ok(true);
    }

    ensure!(
        (6..=7).contains(&args.len()),
        "eval benötigt Modus, Golden-Verzeichnis, Korpus, JSON-Bericht, Markdown-Bericht und für Dense/Hybrid einen Retrieval-Bericht"
    );

    let mode = EvalMode::parse(&args[1])?;
    let golden_dir = PathBuf::from(&args[2]);
    let docs_path = PathBuf::from(&args[3]);
    let json_path = PathBuf::from(&args[4]);
    let markdown_path = PathBuf::from(&args[5]);
    let retrieval_report = args.get(6).map(PathBuf::from);

    if mode == EvalMode::Bm25 {
        ensure!(
            retrieval_report.is_none(),
            "BM25 benötigt keinen externen Retrieval-Bericht"
        );
    } else {
        ensure!(
            retrieval_report.is_some(),
            "{} benötigt den vollständigen JSON-Bericht aus 'dl-knowledge hybrid bench'",
            mode.label()
        );
    }

    let cases = load_golden_cases(&golden_dir, &docs_path)?;
    let results = if mode == EvalMode::Bm25 {
        run_bm25(&cases, &docs_path)?
    } else {
        let report_path = retrieval_report
            .as_deref()
            .context("Retrieval-Bericht fehlt")?;
        run_hybrid_report(mode, &cases, report_path)?
    };
    let report = build_report(mode, &cases, results)?;
    write_report(&report, &json_path, &markdown_path)?;
    println!("{}", serde_json::to_string(&report.metrics)?);
    Ok(true)
}

fn print_help() {
    println!(
        "dl-knowledge eval <bm25|dense|hybrid|hybrid-reranker> <eval-verzeichnis> <public-korpus> <bericht.json> <bericht.md> [hybrid-bench.json]\n\
BM25 läuft direkt gegen denselben Korpus wie der Dienst. Dense, Hybrid und Hybrid + Reranker lesen den vollständigen JSON-Bericht des Paket-C-Kommandos 'dl-knowledge hybrid bench'. Der Harness selbst ruft kein LLM auf."
    );
}

fn load_golden_cases(golden_dir: &Path, docs_path: &Path) -> Result<Vec<GoldenCase>> {
    let mut files = fs::read_dir(golden_dir)
        .with_context(|| format!("Golden-Verzeichnis lesen: {}", golden_dir.display()))?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| {
        path.is_file() && path.extension() == Some(OsStr::new("json"))
    });
    files.sort();

    let names = files
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let expected = GOLDEN_FILES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    ensure!(
        names == expected,
        "Golden-Dateien stimmen nicht: erwartet {expected:?}, gefunden {names:?}"
    );

    let mut cases = Vec::new();
    let mut questions = HashSet::new();
    for file in files {
        let raw = fs::read_to_string(&file)
            .with_context(|| format!("Golden-Datei lesen: {}", file.display()))?;
        let parsed = serde_json::from_str::<Vec<GoldenCase>>(&raw)
            .with_context(|| format!("Golden-Datei parsen: {}", file.display()))?;
        ensure!(
            !parsed.is_empty(),
            "Golden-Datei ist leer: {}",
            file.display()
        );

        for case in parsed {
            validate_case(&case, docs_path)?;
            ensure!(
                questions.insert(case.question.trim().to_string()),
                "Doppelte Golden-Frage: {:?}",
                case.question
            );
            cases.push(case);
        }
    }

    ensure!(
        cases.len() >= BASELINE_GOLDEN_CASES,
        "Golden-Suite hat {} Fälle, erwartet mindestens {BASELINE_GOLDEN_CASES}",
        cases.len()
    );
    Ok(cases)
}

fn validate_case(case: &GoldenCase, docs_path: &Path) -> Result<()> {
    ensure!(!case.question.trim().is_empty(), "Golden-Frage ist leer");
    if case.answerable {
        ensure!(
            !case.expected_sources.is_empty()
                && !case.context_terms.is_empty()
                && !case.answer_terms.is_empty(),
            "Antwortbarer Fall braucht Quellen, Kontext- und Antwortterme: {:?}",
            case.question
        );
    } else {
        ensure!(
            case.expected_sources.is_empty()
                && case.context_terms.is_empty()
                && case.answer_terms.is_empty(),
            "Nicht antwortbarer Fall darf keine Quellen, Kontext- oder Antwortterme haben: {:?}",
            case.question
        );
    }

    if let Some(origin) = &case.herkunft {
        ensure!(
            origin == "synthetisch",
            "Unbekannte Herkunft {:?} bei {:?}",
            origin,
            case.question
        );
    }

    for source in &case.expected_sources {
        let relative = Path::new(source);
        ensure!(!relative.is_absolute(), "Absolute Golden-Quelle: {source:?}");
        ensure!(
            relative.extension() == Some(OsStr::new("html")),
            "Golden-Quelle ist kein HTML: {source:?}"
        );
        ensure!(
            !relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)),
            "Golden-Quelle enthält ParentDir: {source:?}"
        );
        ensure!(
            !relative.components().any(|part| matches!(
                part,
                std::path::Component::Normal(segment)
                    if segment.to_string_lossy().eq_ignore_ascii_case("internal")
            )),
            "Golden-Quelle verweist auf internal: {source:?}"
        );
        ensure!(
            docs_path.join(relative).is_file(),
            "Golden-Quelle fehlt im Korpus: {source:?}"
        );
    }
    Ok(())
}

fn run_bm25(cases: &[GoldenCase], docs_path: &Path) -> Result<Vec<CaseResult>> {
    let knowledge = load_corpus(docs_path)?;
    let stats = knowledge.source_stats();
    ensure!(!knowledge.chunks.is_empty(), "Korpus ist leer");
    ensure!(stats.html_sources > 0, "Korpus enthält keine HTML-Quellen");
    ensure!(
        stats.non_html_sources == 0 && stats.internal_sources == 0,
        "Eval-Korpus muss ausschließlich öffentliche HTML-Quellen enthalten"
    );

    Ok(cases
        .iter()
        .map(|case| evaluate_bm25_case(case, &knowledge))
        .collect())
}

fn evaluate_bm25_case(case: &GoldenCase, knowledge: &KnowledgeBase) -> CaseResult {
    let chunks = knowledge
        .search(&case.question, RETRIEVAL_LIMIT)
        .into_iter()
        .map(|(chunk, _score)| chunk)
        .collect::<Vec<Chunk>>();
    let ranked_paths = chunks
        .iter()
        .map(|chunk| chunk.path.clone())
        .collect::<Vec<_>>();
    let candidates = candidates_for(&chunks);
    let retrieval_answerable = candidates
        .iter()
        .any(|candidate| candidate_is_relevant(&case.question, candidate));
    case_result(case, ranked_paths, retrieval_answerable)
}

fn run_hybrid_report(
    mode: EvalMode,
    cases: &[GoldenCase],
    report_path: &Path,
) -> Result<Vec<CaseResult>> {
    let raw = fs::read_to_string(report_path)
        .with_context(|| format!("Retrieval-Bericht lesen: {}", report_path.display()))?;
    let report = serde_json::from_str::<HybridBenchReport>(&raw)
        .with_context(|| format!("Retrieval-Bericht parsen: {}", report_path.display()))?;
    ensure!(
        report.case_results.len() == cases.len(),
        "Retrieval-Bericht hat {} statt {} Golden-Fälle. Paket C muss denselben Golden-Stand messen.",
        report.case_results.len(),
        cases.len()
    );

    report
        .case_results
        .iter()
        .zip(cases)
        .map(|(backend, golden)| {
            ensure!(
                backend.question == golden.question
                    && backend.answerable == golden.answerable
                    && backend.expected_sources == golden.expected_sources,
                "Retrieval-Bericht passt nicht zum Golden-Fall {:?}",
                golden.question
            );

            let retrieval = match mode {
                EvalMode::Dense => &backend.dense_only,
                EvalMode::Hybrid => &backend.hybrid,
                EvalMode::HybridReranker => &backend.reranked,
                EvalMode::Bm25 => unreachable!(),
            };
            Ok(case_result(
                golden,
                retrieval.ranked_paths.clone(),
                retrieval.relevant_candidates > 0,
            ))
        })
        .collect()
}

fn case_result(
    case: &GoldenCase,
    ranked_paths: Vec<String>,
    retrieval_answerable: bool,
) -> CaseResult {
    let first_relevant_rank = if case.answerable {
        ranked_paths
            .iter()
            .position(|path| case.expected_sources.contains(path))
            .map(|index| index + 1)
    } else {
        None
    };

    CaseResult {
        question: case.question.clone(),
        answerable: case.answerable,
        expected_sources: case.expected_sources.clone(),
        ranked_paths,
        first_relevant_rank,
        retrieval_answerable,
    }
}

fn build_report(
    mode: EvalMode,
    cases: &[GoldenCase],
    results: Vec<CaseResult>,
) -> Result<EvalReport> {
    ensure!(cases.len() == results.len(), "Fallanzahl passt nicht");
    let answerable = results
        .iter()
        .filter(|case| case.answerable)
        .collect::<Vec<_>>();
    let unanswerable = results
        .iter()
        .filter(|case| !case.answerable)
        .collect::<Vec<_>>();
    ensure!(!answerable.is_empty(), "Keine antwortbaren Golden-Fälle");
    ensure!(
        !unanswerable.is_empty(),
        "Keine unbeantwortbaren Golden-Fälle"
    );

    let hit_rate = |k: usize| {
        answerable
            .iter()
            .filter(|case| {
                case.first_relevant_rank
                    .is_some_and(|rank| rank <= k)
            })
            .count() as f64
            / answerable.len() as f64
    };
    let mrr = answerable
        .iter()
        .map(|case| {
            case.first_relevant_rank
                .map(|rank| 1.0 / rank as f64)
                .unwrap_or(0.0)
        })
        .sum::<f64>()
        / answerable.len() as f64;
    let abstained = unanswerable
        .iter()
        .filter(|case| !case.retrieval_answerable)
        .count();

    Ok(EvalReport {
        mode,
        generated_unix_seconds: now_unix()?,
        golden_cases: cases.len(),
        synthetic_cases: cases
            .iter()
            .filter(|case| case.herkunft.as_deref() == Some("synthetisch"))
            .count(),
        retrieval_limit: RETRIEVAL_LIMIT,
        metrics: Metrics {
            answerable_cases: answerable.len(),
            unanswerable_cases: unanswerable.len(),
            recall_at_1: hit_rate(1),
            recall_at_3: hit_rate(3),
            recall_at_5: hit_rate(5),
            mrr,
            citation_correctness: hit_rate(RETRIEVAL_LIMIT),
            abstain_rate: abstained as f64 / unanswerable.len() as f64,
            false_answer_rate: (unanswerable.len() - abstained) as f64
                / unanswerable.len() as f64,
        },
        cases: results,
    })
}

fn write_report(report: &EvalReport, json_path: &Path, markdown_path: &Path) -> Result<()> {
    if let Some(parent) = json_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = markdown_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(json_path, serde_json::to_vec_pretty(report)?)
        .with_context(|| format!("JSON-Bericht schreiben: {}", json_path.display()))?;
    fs::write(markdown_path, markdown(report))
        .with_context(|| format!("Markdown-Bericht schreiben: {}", markdown_path.display()))?;
    Ok(())
}

fn markdown(report: &EvalReport) -> String {
    let metrics = &report.metrics;
    format!(
        "# Eval: {}\n\nGolden-Fälle: {} (davon synthetisch: {}). Retrieval-Limit: {}. Kein LLM-Aufruf.\n\n| Modus | Recall@1 | Recall@3 | Recall@5 | MRR | Citation-Correctness | Abstain-Rate | Fehl-Antwort-Rate |\n|---|---:|---:|---:|---:|---:|---:|---:|\n| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n\nAntwortbare Fälle: {}. Unbeantwortbare Fälle: {}. Citation-Correctness bedeutet hier: mindestens eine erwartete Quelle liegt unter den ersten {} Retrieval-Quellen. Abstain- und Fehl-Antwort-Rate werden im Retrieval-Harness an der bestehenden lexikalischen Relevanzprüfung gemessen, nicht an einer LLM-Generation.\n",
        report.mode.label(),
        report.golden_cases,
        report.synthetic_cases,
        report.retrieval_limit,
        report.mode.label(),
        metrics.recall_at_1,
        metrics.recall_at_3,
        metrics.recall_at_5,
        metrics.mrr,
        metrics.citation_correctness,
        metrics.abstain_rate,
        metrics.false_answer_rate,
        metrics.answerable_cases,
        metrics.unanswerable_cases,
        report.retrieval_limit,
    )
}

fn now_unix() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Systemzeit liegt vor Unix-Epoch")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(answerable: bool) -> GoldenCase {
        GoldenCase {
            question: if answerable { "A" } else { "B" }.into(),
            answerable,
            expected_sources: if answerable {
                vec!["a.html".into()]
            } else {
                vec![]
            },
            context_terms: if answerable {
                vec!["Kontext".into()]
            } else {
                vec![]
            },
            answer_terms: if answerable {
                vec!["Antwort".into()]
            } else {
                vec![]
            },
            forbidden_terms: vec![],
            herkunft: (!answerable).then(|| "synthetisch".into()),
        }
    }

    #[test]
    fn metriken_trennen_recall_und_abstain() -> Result<()> {
        let cases = vec![golden(true), golden(false)];
        let results = vec![
            case_result(&cases[0], vec!["x.html".into(), "a.html".into()], true),
            case_result(&cases[1], vec!["x.html".into()], false),
        ];
        let report = build_report(EvalMode::Bm25, &cases, results)?;
        assert_eq!(report.metrics.recall_at_1, 0.0);
        assert_eq!(report.metrics.recall_at_3, 1.0);
        assert_eq!(report.metrics.mrr, 0.5);
        assert_eq!(report.metrics.abstain_rate, 1.0);
        assert_eq!(report.metrics.false_answer_rate, 0.0);
        Ok(())
    }

    #[test]
    fn modus_alias_fuer_reranker() -> Result<()> {
        assert_eq!(
            EvalMode::parse("hybrid+reranker")?,
            EvalMode::HybridReranker
        );
        assert!(EvalMode::parse("sonstwas").is_err());
        Ok(())
    }
}
