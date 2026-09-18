use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::{config::Config, pipeline, quality, Models};

pub struct Plan {
    pub rounds: usize,
    pub first: usize,
    pub count: Option<usize>,
}

impl Plan {
    pub fn range(&self, total: usize) -> Result<std::ops::Range<usize>> {
        ensure!((1..=5).contains(&self.rounds), "Ungültige Messauswahl");
        let count = self
            .count
            .unwrap_or_else(|| total.saturating_sub(self.first));
        ensure!(count > 0, "Ungültige Messauswahl");
        let end = self
            .first
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("Messbereich ist zu groß"))?;
        ensure!(
            end <= total,
            "Messbereich liegt außerhalb der vollständigen Golden-Suite"
        );
        Ok(self.first..end)
    }
}

pub async fn run(
    pool: &PgPool,
    models: &mut Models,
    config: &Config,
    eval_dir: &Path,
    output: &Path,
    plan: Plan,
    model_load_ms: f64,
) -> Result<()> {
    ensure!(
        pool.connect_options().get_database() == Some("deadlock_test")
            && std::env::var("TURNIER_TEST_DB_CONFIRM").ok().as_deref() == Some("throwaway-only"),
        "C-Messung nur über scripts/central_test_db.sh"
    );
    ensure!(
        (1..=5).contains(&plan.rounds),
        "Messung benötigt 1 bis 5 Runden"
    );
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM knowledge.index_generations")
        .fetch_one(pool)
        .await?;
    ensure!(existing == 0, "Messung benötigt eine frische Testdatenbank");
    let root = Path::new(crate::DEFAULT_DOCS_PATH).canonicalize()?;
    let knowledge = crate::load_production_corpus(&root)?;
    let records = crate::dense::from_chunks(&knowledge.chunks)?;
    let cases = quality::load(eval_dir, &root)?;
    let corpus_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&records)?));
    let cases_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&cases)?));
    let suite_cases = cases.len();
    let selected = plan.range(suite_cases)?;
    let cases = &cases[selected.clone()];
    let rounds = plan.rounds;
    let mut checkpoint = std::fs::File::create(output.with_extension("jsonl"))?;
    writeln!(
        checkpoint,
        "{}",
        json!({"kind":"header", "config":config, "corpus_hash":corpus_hash,
        "golden_hash":cases_hash, "suite_cases":suite_cases, "range_start":selected.start, "range_end":selected.end,
        "rounds":rounds, "embedding_fingerprint":models.embedder.fingerprint(),
        "reranker_fingerprint":models.reranker.as_ref().map(|model| model.fingerprint())})
    )?;
    let start = Instant::now();
    let built =
        crate::dense::store::build_generation(pool, models.embedder.as_mut(), &records).await?;
    let index_ms = start.elapsed().as_secs_f64() * 1000.0;
    crate::dense::store::activate(pool, built.index_generation, None).await?;
    let deadline = || Instant::now() + Duration::from_millis(config.timeout_ms);
    for (_, _, case) in cases.iter().take(3) {
        pipeline::run(pool, models, &knowledge, &case.question, config, deadline()).await?;
    }
    let mut bm25_samples = Vec::new();
    let mut hybrid_samples = Vec::new();
    let mut reranked_samples = Vec::new();
    let mut rerank_samples = Vec::new();
    let mut rows = Vec::new();
    for round in 0..rounds {
        for (position, (file, row, case)) in cases.iter().enumerate() {
            let start = Instant::now();
            let bm25 = knowledge
                .search(&case.question, config.output_k)
                .into_iter()
                .map(|(chunk, _)| chunk)
                .collect::<Vec<_>>();
            bm25_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            let result =
                pipeline::run(pool, models, &knowledge, &case.question, config, deadline()).await?;
            hybrid_samples.push(result.timing.without_rerank_ms);
            reranked_samples.push(result.timing.total_ms);
            rerank_samples.push(result.timing.rerank_ms);
            if round == 0 {
                let fused = result
                    .fused
                    .iter()
                    .take(config.output_k)
                    .map(|hit| knowledge.chunks[hit.index].clone())
                    .collect::<Vec<_>>();
                let reranked = result
                    .reranked
                    .iter()
                    .take(config.output_k)
                    .map(|hit| knowledge.chunks[hit.index].clone())
                    .collect::<Vec<_>>();
                let dense_only = result
                    .dense
                    .iter()
                    .filter(|index| !result.bm25.contains(index))
                    .map(|index| knowledge.chunks[*index].clone())
                    .collect::<Vec<_>>();
                rows.push(json!({"suite_position":selected.start+position+1,"file":file, "row":row, "question":case.question, "answerable":case.answerable,
                    "expected_sources":case.expected_sources, "bm25":quality::measure(case, &bm25),
                    "hybrid":quality::measure(case, &fused), "reranked":quality::measure(case, &reranked),
                    "dense_only":quality::measure(case, &dense_only), "timing":result.timing}));
            }
            writeln!(
                checkpoint,
                "{}",
                json!({"kind":"sample", "round":round+1,
                "suite_position":selected.start+position+1, "bm25_ms":bm25_samples.last(),
                "hybrid_ms":hybrid_samples.last(), "reranked_ms":reranked_samples.last(), "rerank_ms":rerank_samples.last(),
                "case_result":if round == 0 { rows.last() } else { None }})
            )?;
            checkpoint.flush()?;
            if position % 16 == 0 || position + 1 == cases.len() {
                std::fs::write(
                    output.with_extension("progress.json"),
                    serde_json::to_vec(
                        &json!({"round":round+1,"completed":position+1,"cases":cases.len()}),
                    )?,
                )?;
            }
        }
    }
    let report = json!({
        "timestamp_unix":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "config":config, "corpus_root":root, "corpus_hash":corpus_hash, "golden_hash":cases_hash,
        "chunks":records.len(), "html_sources":knowledge.source_stats().html_sources, "cases":cases.len(), "rounds":rounds,
        "suite_cases":suite_cases, "range_start":selected.start, "range_end":selected.end, "complete_suite":cases.len()==suite_cases,
        "embedding_fingerprint":models.embedder.fingerprint(), "reranker_fingerprint":models.reranker.as_ref().map(|model| model.fingerprint()),
        "model_load_ms":model_load_ms, "index_ms":index_ms,
        "bm25":summary(&rows,"bm25",&bm25_samples), "hybrid":summary(&rows,"hybrid",&hybrid_samples), "reranked":summary(&rows,"reranked",&reranked_samples),
        "rerank_p50_ms":percentile(&rerank_samples,0.5), "rerank_p95_ms":percentile(&rerank_samples,0.95),
        "case_results":rows, "bm25_samples_ms":bm25_samples, "hybrid_samples_ms":hybrid_samples,
        "reranked_samples_ms":reranked_samples, "rerank_samples_ms":rerank_samples,
        "rss_kib":memory("VmRSS:"), "peak_rss_kib":memory("VmHWM:"),
        "generation_calls":0, "latency_scope":"loaded models; pipeline only, without HTTP/worker dispatch and LLM; shared prefix measured before reranking",
        "quality_scope":"retrieval and feasible grounded selections; negative candidates are not live answers; no faithfulness or live abstain claim"
    });
    let temporary = output.with_extension("tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&report)?)?;
    std::fs::rename(temporary, output)?;
    writeln!(
        checkpoint,
        "{}",
        json!({"kind":"complete", "cases":cases.len(),"rounds":rounds})
    )?;
    println!(
        "{}",
        json!({"report":output,"cases":cases.len(),"rounds":rounds})
    );
    Ok(())
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() as f64 * fraction).ceil() as usize).saturating_sub(1)]
}

fn summary(rows: &[Value], mode: &str, samples: &[f64]) -> Value {
    let positives = rows
        .iter()
        .filter(|row| row["answerable"] == true)
        .collect::<Vec<_>>();
    let negatives = rows
        .iter()
        .filter(|row| row["answerable"] == false)
        .collect::<Vec<_>>();
    let count = |field: &str| {
        positives
            .iter()
            .filter(|row| row[mode][field] == true)
            .count()
    };
    let mean = |field: &str| {
        positives
            .iter()
            .map(|row| row[mode][field].as_f64().unwrap_or(0.0))
            .sum::<f64>()
            / positives.len() as f64
    };
    json!({"positive_cases":positives.len(), "negative_cases":negatives.len(), "hit_cases":count("hit"),
        "source_recall_at_k":mean("source_recall"), "mrr_at_k":mean("reciprocal_rank"),
        "grounded_selection_possible_cases":count("grounded_selection_possible"),
        "expected_source_lost_to_relevance_cases":count("expected_source_lost_to_relevance"),
        "answer_terms_lost_to_relevance_cases":count("answer_terms_lost_to_relevance"),
        "positive_cases_without_relevant_candidates":positives.iter().filter(|row| row[mode]["relevant_candidates"] == 0).count(),
        "negative_cases_without_relevant_candidates":negatives.iter().filter(|row| row[mode]["relevant_candidates"] == 0).count(),
        "missing_context_cases":positives.iter().filter(|row| row[mode]["missing_context_terms"].as_array().is_some_and(|terms| !terms.is_empty())).count(),
        "forbidden_context_cases":rows.iter().filter(|row| row[mode]["forbidden_context_terms"].as_array().is_some_and(|terms| !terms.is_empty())).count(),
        "query_samples":samples.len(), "query_p50_ms":percentile(samples,0.5), "query_p95_ms":percentile(samples,0.95)})
}

fn memory(key: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with(key))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
