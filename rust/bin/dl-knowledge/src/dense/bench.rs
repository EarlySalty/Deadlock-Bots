use std::collections::HashSet;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;

use super::{from_chunks, sha256, store, Embedder};

#[derive(Deserialize, Serialize)]
struct Case {
    question: String,
    answerable: bool,
    expected_sources: Vec<String>,
}

pub async fn run(
    pool: &PgPool,
    embedder: &mut dyn Embedder,
    backend: &str,
    model_load_ms: f64,
    eval_dir: &Path,
    output: &Path,
) -> Result<()> {
    ensure!(
        pool.connect_options().get_database() == Some("deadlock_test"),
        "Messung nur in deadlock_test über scripts/central_test_db.sh"
    );
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM knowledge.index_generations")
        .fetch_one(pool)
        .await?;
    ensure!(
        existing == 0,
        "Messung benötigt eine frische Datenbank ohne Embedding-Cache"
    );
    let load_start = Instant::now();
    let root = Path::new(crate::DEFAULT_DOCS_PATH).canonicalize()?;
    let knowledge = crate::load_production_corpus(&root)?;
    let chunks = from_chunks(&knowledge.chunks)?;
    let corpus_load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let corpus_hash = sha256(&serde_json::to_vec(&chunks)?);
    let html_sources = chunks
        .iter()
        .map(|chunk| &chunk.doc_path)
        .collect::<HashSet<_>>()
        .len();
    let cases = load_cases(eval_dir)?;
    let selected_cases_hash = sha256(&serde_json::to_vec(&cases)?);
    let index_start = Instant::now();
    let built = store::build_generation(pool, embedder, &chunks).await?;
    let index_ms = index_start.elapsed().as_secs_f64() * 1000.0;
    store::activate(pool, built.index_generation, None).await?;
    for (_, _, case) in cases.iter().take(3) {
        store::dense_search(pool, embedder, &case.question, 6).await?;
    }
    let mut embedding_times = Vec::new();
    let mut total_times = Vec::new();
    let mut rows = Vec::new();
    let mut hit1 = 0.0;
    let mut hit6 = 0.0;
    let mut recall6 = 0.0;
    let mut mrr6 = 0.0;
    for round in 0..5 {
        for (file, row, case) in &cases {
            let start = Instant::now();
            let vector = embedder.embed_queries(std::slice::from_ref(&case.question))?;
            ensure!(vector.len() == 1, "Query-Embedding-Anzahl ist ungültig");
            embedding_times.push(start.elapsed().as_secs_f64() * 1000.0);
            let hits = store::search_vector(pool, embedder.fingerprint(), &vector[0], 6).await?;
            total_times.push(start.elapsed().as_secs_f64() * 1000.0);
            if round != 0 {
                continue;
            }
            let expected: HashSet<&str> =
                case.expected_sources.iter().map(String::as_str).collect();
            let retrieved: HashSet<&str> = hits.iter().map(|hit| hit.doc_path.as_str()).collect();
            let first = hits
                .iter()
                .position(|hit| expected.contains(hit.doc_path.as_str()));
            hit1 += f64::from(first == Some(0));
            hit6 += f64::from(first.is_some());
            recall6 += expected.intersection(&retrieved).count() as f64 / expected.len() as f64;
            mrr6 += first.map_or(0.0, |rank| 1.0 / (rank + 1) as f64);
            rows.push(json!({"file": file, "row": row, "question": case.question, "expected_sources": case.expected_sources,
                "ranked_paths": hits.iter().map(|hit| &hit.doc_path).collect::<Vec<_>>(), "first_relevant_rank": first.map(|rank| rank + 1)}));
        }
    }
    let report = json!({
        "backend": backend, "model_fingerprint": embedder.fingerprint(),
        "timestamp_unix": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "corpus_root": root, "corpus_hash": corpus_hash, "html_sources": html_sources, "chunks": chunks.len(),
        "selected_cases_hash": selected_cases_hash, "cases": cases.len(), "rounds": 5, "query_samples": total_times.len(),
        "model_load_ms": model_load_ms, "corpus_load_ms": corpus_load_ms,
        "index_ms": index_ms, "index_computed": built.computed, "index_reused": built.reused,
        "query_embedding_p50_ms": percentile(&embedding_times, 0.5), "query_embedding_p95_ms": percentile(&embedding_times, 0.95),
        "query_total_p50_ms": percentile(&total_times, 0.5), "query_total_p95_ms": percentile(&total_times, 0.95),
        "rss_kib": proc_memory("VmRSS:"), "peak_rss_kib": proc_memory("VmHWM:"),
        "binary_bytes": std::fs::metadata(std::env::current_exe()?)?.len(),
        "hit_at_1": hit1 / cases.len() as f64, "hit_at_6": hit6 / cases.len() as f64,
        "source_recall_at_6": recall6 / cases.len() as f64, "mrr_at_6": mrr6 / cases.len() as f64,
        "case_results": rows, "embedding_samples_ms": embedding_times, "total_samples_ms": total_times
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"report": output, "backend": backend, "cases": cases.len(), "chunks": chunks.len()})
    );
    Ok(())
}

fn load_cases(dir: &Path) -> Result<Vec<(String, usize, Case)>> {
    let mut files = std::fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    files.sort();
    ensure!(
        files.len() == 6,
        "Messung erwartet die sechs öffentlichen Eval-Dateien"
    );
    let mut selected = Vec::new();
    for path in files {
        let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&path)?)?;
        let name = path
            .file_name()
            .context("Eval-Dateiname fehlt")?
            .to_string_lossy()
            .into_owned();
        let entries = cases
            .into_iter()
            .enumerate()
            .filter(|(_, case)| case.answerable)
            .take(5)
            .collect::<Vec<_>>();
        ensure!(
            entries.len() == 5,
            "Weniger als fünf beantwortbare Fälle in {name}"
        );
        for (row, case) in entries {
            ensure!(
                !case.question.trim().is_empty() && !case.expected_sources.is_empty(),
                "Ungültiger Eval-Fall"
            );
            selected.push((name.clone(), row + 1, case));
        }
    }
    ensure!(selected.len() == 30, "Messung benötigt genau 30 Fälle");
    Ok(selected)
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() as f64 * fraction).ceil() as usize).saturating_sub(1)]
}

fn proc_memory(key: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with(key))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
