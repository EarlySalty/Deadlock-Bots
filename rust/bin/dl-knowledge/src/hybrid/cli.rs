use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicBench {
    settings: crate::server_config::HybridSettings,
    public_snapshot: std::path::PathBuf,
    evaluation: std::path::PathBuf,
    report: std::path::PathBuf,
    #[serde(default)]
    first: usize,
    count: Option<usize>,
    #[serde(default)]
    holdout: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicShadow {
    settings: Option<crate::server_config::HybridSettings>,
    public_snapshot: std::path::PathBuf,
    bind: std::net::SocketAddr,
}

async fn public_shadow(path: &Path) -> Result<()> {
    let input: PublicShadow = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        input.bind.ip().is_loopback() && input.bind.port() != 0,
        "Shadow benötigt festen Loopback-Port"
    );
    let root = input.public_snapshot.canonicalize()?;
    let knowledge = crate::load_production_corpus(&root)?;
    let listener = tokio::net::TcpListener::bind(input.bind)
        .await
        .context("Shadow-Port reservieren")?;
    let runtime = if let Some(settings) = &input.settings {
        settings.validate()?;
        let address = settings
            .database_url
            .as_deref()
            .context("Shadow benötigt ausdrückliche lokale Testdatenbank")?;
        let url = url::Url::parse(address)?;
        ensure!(
            url.path().starts_with("/dl_knowledge_eval_") && url.path().len() > 19,
            "Shadow darf nur eigene Testdatenbank verwenden"
        );
        let runtime = super::Runtime::from_config(settings).await?;
        let prepared = runtime.prepare(&knowledge).await?;
        runtime.activate(prepared).await?;
        Some(runtime)
    } else {
        // The unchanged lexical route is needed as a fair same-corpus baseline.
        None
    };
    let state = crate::AppState {
        reload_gate: Default::default(),
        docs_path: input.public_snapshot,
        knowledge: std::sync::Arc::new(tokio::sync::RwLock::new(knowledge)),
        generator: None,
        hybrid: runtime,
    };
    println!(
        "{}",
        serde_json::json!({"shadow":"ready","bind":input.bind})
    );
    axum::serve(listener, crate::router_with_mode(state, true))
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::warn!(%error, "Shadow-Stoppsignal nicht verfügbar");
            }
        })
        .await
        .context("Shadow-HTTP-Dienst")
}

async fn public_bench(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).context("Messkonfiguration lesen")?;
    let input: PublicBench = serde_json::from_slice(&bytes)?;
    input.settings.validate()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(
            input
                .settings
                .database_url
                .as_deref()
                .context("Offline-Messung benötigt explizite lokale Testdatenbank")?,
        )
        .await?;
    let name = pool
        .connect_options()
        .get_database()
        .map(str::to_owned)
        .unwrap_or_default();
    ensure!(
        name.starts_with("dl_knowledge_eval_") && name.len() > 18,
        "Öffentliche Messung benötigt eine eigene dl_knowledge_eval_* Wegwerfdatenbank"
    );
    let start = Instant::now();
    let mut models = Models::load_explicit(&input.settings)?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    bench::run_snapshot(
        &pool,
        &mut models,
        &input.settings.options,
        &input.evaluation,
        &input.report,
        bench::Plan {
            holdout: input.holdout,
            rounds: 1,
            first: input.first,
            count: input.count,
        },
        load_ms,
        &input.public_snapshot.canonicalize()?,
    )
    .await?;
    pool.close().await;
    Ok(())
}

use super::{bench, config::Config, model_path, rerank, Models};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckInput {
    question: String,
    documents: Vec<String>,
}

pub async fn run() -> Result<bool> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("public-shadow") {
        ensure!(
            args.len() == 2,
            "public-shadow benötigt genau eine JSON-Konfiguration"
        );
        public_shadow(Path::new(&args[1])).await?;
        return Ok(true);
    }
    if args.first().map(String::as_str) == Some("public-bench") {
        ensure!(
            args.len() == 2,
            "public-bench benötigt genau eine JSON-Konfiguration"
        );
        public_bench(Path::new(&args[1])).await?;
        return Ok(true);
    }
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
        holdout: false,
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
