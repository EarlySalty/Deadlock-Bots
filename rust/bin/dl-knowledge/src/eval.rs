use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

    fn backend_field(self) -> Option<&'static str> {
        match self {
            Self::Bm25 => None,
            Self::Dense => Some("dense_only"),
            Self::Hybrid => Some("hybrid"),
            Self::HybridReranker => Some("reranked"),
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
struct BackendReport {
    case_results: Vec<BackendCase>,
}

#[derive(Debug, Deserialize)]
struct BackendCase {
    answerable: bool,
    question: String,
    expected_sources: Vec<String>,
    dense_only: BackendRetrieval,
    hybrid: BackendRetrieval,
    reranked: BackendRetrieval,
}

#[derive(Debug, Deserialize)]
struct BackendRetrieval {
    ranked_paths: Vec<String>,
    relevant_candidates: usize,
}

pub fn run() -> Result<bool> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("eval") {
        return Ok(false);
    }
    if args.get(1).is_none() || matches!(args.get(1).map(String::as_str), Some("help" | "--help")) {
        print_help();
        return Ok(true);
    }
    ensure!(
        (6..=7).contains(&args.len()),
        "eval benötigt Modus, Golden-Verzeichnis, Korpus, JSON-Bericht, Markdown-Bericht und optional Backend-Binary"
    );
    let mode = EvalMode::parse(&args[1])?;
    let golden_dir = PathBuf::from(&args[2]);
    let docs_path = PathBuf::from(&args[3]);
    let json_path = PathBuf::from(&args[4]);
    let markdown_path = PathBuf::from(&args[5]);
    let backend_bin = args.get(6).map(PathBuf::from);

    let cases = load_golden_cases(&golden_dir, &docs_path)?;
    let results = match mode {
        EvalMode::Bm25 => run_bm25(&cases, &docs_path)?,
        _ => run_backend(mode, &cases, &golden_dir, backend_bin.as_deref())?,
    };
    let report = build_report(mode, &cases, results)?;
    write_report(&report, &json_path, &markdown_path)?;
    println!("{}", serde_json::to_string(&report.metrics)?);
    Ok(true)
}

fn print_help() {
    println!(
        "dl-knowledge eval <bm25|dense|hybrid|hybrid-reranker> <eval-verzeichnis> <public-korpus> <bericht.json> <bericht.md> [backend-binary]\n\
BM25 läuft direkt und ohne Dienst. Dense/Hybrid verwenden das vorhandene Paket-C-Backendkommando 'hybrid bench' ohne LLM; backend-binary ist dort Pflicht.\n\
Für Backend-Modi muss der Aufruf in derselben frischen deadlock_test-Umgebung laufen, die Paket C für 'hybrid bench' verlangt."
    );
}

fn load_golden_cases(golden_dir: &Path, docs_path: &Path) -> Result<Vec<GoldenCase>> {
    let mut files = fs::read_dir(golden_dir)
        .with_context(|| format!("Golden-Verzeichnis lesen: {}", golden_dir.display()))?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"));
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
    ensure!(
        case.forbidden_terms
            .iter()
            .all(|term| !term.trim().is_empty()),
        "Leerer verbotener Term bei {:?}",
        case.question
    );
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
        ensure!(
            !relative.is_absolute(),
            "Absolute Golden-Quelle: {source:?}"
        );
        ensure!(
            relative.extension().is_some_and(|ext| ext == "html"),
            "Golden-Quelle ist kein HTML: {source:?}"
        );
        ensure!(
            !relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)),
            "Golden-Quelle enthält ParentDir: {source:?}"
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
        "Eval-Korpus muss dem öffentlichen Produktionskorpus entsprechen"
    );

    Ok(cases
        .iter()
        .map(|case| evaluate_bm25_case(case, &knowledge))
        .collect())
}

fn evaluate_bm25_case(case: &GoldenCase, knowledge: &KnowledgeBase) -> CaseResult {
    let ranked = knowledge.search(&case.question, RETRIEVAL_LIMIT);
    let chunks = ranked
        .into_iter()
        .map(|(chunk, _)| chunk)
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

fn run_backend(
    mode: EvalMode,
    cases: &[GoldenCase],
    golden_dir: &Path,
    backend_bin: Option<&Path>,
) -> Result<Vec<CaseResult>> {
    let backend_bin = backend_bin.context("Backend-Binary fehlt für Dense/Hybrid-Eval")?;
    let temp = std::env::temp_dir().join(format!(
        "dl-knowledge-eval-{}-{}.json",
        std::process::id(),
        now_unix()?
    ));
    let rerank = matches!(mode, EvalMode::HybridReranker);
    let options = std::env::var("DL_KNOWLEDGE_EVAL_HYBRID_OPTIONS").unwrap_or_else(|_| {
        format!(
            "{{\"timeout_ms\":10000,\"fusion_k\":12,\"rerank\":{}}}",
            if rerank { "true" } else { "false" }
        )
    });
    let status = Command::new(backend_bin)
        .args([
            "hybrid",
            "bench",
            &golden_dir.to_string_lossy(),
            &temp.to_string_lossy(),
            "1",
        ])
        .env("DL_KNOWLEDGE_HYBRID_OPTIONS", options)
        .status()
        .with_context(|| format!("Eval-Backend starten: {}", backend_bin.display()))?;
    ensure!(
        status.success(),
        "Eval-Backend ist mit {status} fehlgeschlagen"
    );

    let raw = fs::read_to_string(&temp)
        .with_context(|| format!("Backend-Bericht lesen: {}", temp.display()))?;
    let _ = fs::remove_file(&temp);
    let report: BackendReport = serde_json::from_str(&raw).context("Backend-Bericht parsen")?;
    ensure!(
        report.case_results.len() == cases.len(),
        "Backend-Bericht hat {} statt {} Fälle",
        report.case_results.len(),
        cases.len()
    );
    let field = mode.backend_field().context("Backend-Feld fehlt")?;
    report
        .case_results
        .iter()
        .zip(cases)
        .map(|(backend, golden)| {
            ensure!(
                backend.question == golden.question
                    && backend.answerable == golden.answerable
                    && backend.expected_sources == golden.expected_sources,
                "Backend-Golden-Fall stimmt nicht mit D-Harness überein: {:?}",
                golden.question
            );
            let retrieval = match field {
                "dense_only" => &backend.dense_only,
                "hybrid" => &backend.hybrid,
                "reranked" => &backend.reranked,
                _ => unreachable!(),
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
            .filter(|case| case.first_relevant_rank.is_some_and(|rank| rank <= k))
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
            false_answer_rate: (unanswerable.len() - abstained) as f64 / unanswerable.len() as f64,
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
    let m = &report.metrics;
    format!(
        "# Eval: {}\n\nGolden-Fälle: {} (davon synthetisch: {}). Retrieval-Limit: {}. Kein LLM-Aufruf.\n\n| Modus | Recall@1 | Recall@3 | Recall@5 | MRR | Citation-Correctness | Abstain-Rate | Fehl-Antwort-Rate |\n|---|---:|---:|---:|---:|---:|---:|---:|\n| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n\nAntwortbare Fälle: {}. Unbeantwortbare Fälle: {}. Citation-Correctness bedeutet: mindestens eine erwartete Quelle liegt unter den ersten {} Retrieval-Quellen. Abstain/Fehl-Antwort werden ohne Generation an der bestehenden lexikalischen Relevanzprüfung gemessen.\n",
        report.mode.label(),
        report.golden_cases,
        report.synthetic_cases,
        report.retrieval_limit,
        report.mode.label(),
        m.recall_at_1,
        m.recall_at_3,
        m.recall_at_5,
        m.mrr,
        m.citation_correctness,
        m.abstain_rate,
        m.false_answer_rate,
        m.answerable_cases,
        m.unanswerable_cases,
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

    #[test]
    fn metriken_trennen_recall_und_abstain() -> Result<()> {
        let cases = vec![
            GoldenCase {
                question: "A".into(),
                answerable: true,
                expected_sources: vec!["a.html".into()],
                context_terms: vec!["x".into()],
                answer_terms: vec!["y".into()],
                forbidden_terms: vec![],
                herkunft: None,
            },
            GoldenCase {
                question: "B".into(),
                answerable: false,
                expected_sources: vec![],
                context_terms: vec![],
                answer_terms: vec![],
                forbidden_terms: vec![],
                herkunft: Some("synthetisch".into()),
            },
        ];
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
