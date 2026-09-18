use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;

use super::{bench, config::Config, model_path, rerank, Models};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckInput {
    question: String,
    documents: Vec<String>,
}

pub async fn run() -> Result<bool> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("hybrid") {
        return Ok(false);
    }
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    if matches!(command, "help" | "--help") {
        println!("dl-knowledge hybrid check < paare.json\ndl-knowledge hybrid bench <eval-verzeichnis> <bericht.json> [runden=1] [start=0] [anzahl=alle]\ncheck benötigt nur lokale Reranker-Dateien; bench ausschließlich in einer frischen Wegwerf-Testdatenbank.\nOptionen: DL_KNOWLEDGE_HYBRID_OPTIONS als JSON; Modelle: DL_KNOWLEDGE_MODELS und DL_KNOWLEDGE_RERANK_MODELS.\nDer Dienstschalter DL_KNOWLEDGE_HYBRID bleibt standardmäßig false.");
        return Ok(true);
    }
    ensure!(
        matches!(command, "check" | "bench"),
        "Unbekanntes Hybrid-Subcommand"
    );
    let options = std::env::var("DL_KNOWLEDGE_HYBRID_OPTIONS").ok();
    let config = Config::from_values(Some("true"), options.as_deref())?
        .context("Hybrid-Konfiguration fehlt")?;
    if command == "check" {
        ensure!(
            args.len() == 2,
            "check liest ausschließlich JSON über stdin"
        );
        let mut bytes = Vec::new();
        std::io::stdin().take(131073).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 131072, "Reranker-Prüfeingabe ist zu groß");
        let input: CheckInput = serde_json::from_slice(&bytes)?;
        ensure!(
            !input.question.trim().is_empty()
                && input.question.len() <= 4096
                && (1..=100).contains(&input.documents.len()),
            "Ungültige Reranker-Prüfeingabe"
        );
        let mut model = rerank::load(
            &model_path("DL_KNOWLEDGE_RERANK_MODELS", "ms-marco-MiniLM-L6-v2")?,
            &config,
        )?;
        let scores = model.scores(&input.question, &input.documents)?;
        println!(
            "{}",
            serde_json::json!({"model_fingerprint":model.fingerprint(), "scores":scores})
        );
        return Ok(true);
    }
    ensure!(
        (4..=7).contains(&args.len()),
        "bench benötigt Eval-Verzeichnis, Bericht und optional Runden, Start, Anzahl"
    );
    let rounds = args
        .get(4)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(1);
    let first = args
        .get(5)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let count = args
        .get(6)
        .map(|value| value.parse::<usize>())
        .transpose()?;
    let plan = bench::Plan {
        rounds,
        first,
        count,
    };
    let started = Instant::now();
    let mut models = Models::load(&config)?;
    let model_load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let pool = dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?;
    bench::run(
        &pool,
        &mut models,
        &config,
        Path::new(&args[2]),
        Path::new(&args[3]),
        plan,
        model_load_ms,
    )
    .await?;
    pool.close().await;
    Ok(true)
}
