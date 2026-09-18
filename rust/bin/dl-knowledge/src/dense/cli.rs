use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};

use super::{bench, from_chunks, local, store};

pub async fn run() -> Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("dense") {
        return Ok(false);
    }
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    if command == "help" || command == "--help" {
        println!("dl-knowledge dense index [fastembed|candle]\ndl-knowledge dense search [fastembed|candle] [top_k] < frage.txt\ndl-knowledge dense check [fastembed|candle] < frage.txt\ndl-knowledge dense activate <generation> <erwartete-generation|none>\ndl-knowledge dense bench <fastembed|candle> <eval-verzeichnis> <bericht.json>\nModelldateien: DL_KNOWLEDGE_MODELS oder ~/.local/share/dl-knowledge/models/all-MiniLM-L6-v2\nindex aktiviert niemals selbst; activate erst nach Offline-Eval.");
        return Ok(true);
    }
    if command == "activate" {
        ensure!(
            args.len() == 4,
            "activate benötigt Generation und erwarteten bisherigen Stand"
        );
        let generation: i64 = args[2].parse().context("Ungültige Generation")?;
        let expected = if args[3] == "none" {
            None
        } else {
            Some(args[3].parse().context("Ungültiger erwarteter Stand")?)
        };
        let pool = dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?;
        store::activate(&pool, generation, expected).await?;
        println!(
            "{}",
            serde_json::json!({"index_generation": generation, "activated": true})
        );
        pool.close().await;
        return Ok(true);
    }
    ensure!(
        matches!(command, "index" | "search" | "check" | "bench"),
        "Unbekanntes Dense-Subcommand"
    );
    let max_args = match command {
        "bench" => 5,
        "search" => 4,
        _ => 3,
    };
    ensure!(args.len() <= max_args, "Zu viele Argumente");
    let backend = args.get(2).map(String::as_str).unwrap_or(default_backend());
    let root = if let Some(path) = std::env::var_os("DL_KNOWLEDGE_MODELS") {
        PathBuf::from(path)
    } else {
        PathBuf::from(std::env::var_os("HOME").context("HOME fehlt für den lokalen Modellpfad")?)
            .join(".local/share/dl-knowledge/models/all-MiniLM-L6-v2")
    };
    let load_start = Instant::now();
    let mut embedder = local::load(backend, &root)?;
    let model_load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    if command == "check" {
        let query = query_from_stdin()?;
        let embeddings = embedder.embed(&[query])?;
        ensure!(
            embeddings.len() == 1,
            "Embedding-Anzahl passt nicht zur Eingabe"
        );
        super::validate_embedding(&embeddings[0])?;
        println!(
            "{}",
            serde_json::json!({"dimensions": embeddings[0].len(), "model_fingerprint": embedder.fingerprint()})
        );
        return Ok(true);
    }
    let pool = dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?;
    match command {
        "index" => {
            let knowledge =
                crate::load_production_corpus(&PathBuf::from(crate::DEFAULT_DOCS_PATH))?;
            let chunks = from_chunks(&knowledge.chunks)?;
            let result = store::build_generation(&pool, embedder.as_mut(), &chunks).await?;
            println!("{}", serde_json::to_string(&result)?);
        }
        "search" => {
            let top_k = args
                .get(3)
                .map(|value| value.parse::<usize>())
                .transpose()?
                .unwrap_or(6);
            let query = query_from_stdin()?;
            let hits = store::dense_search(&pool, embedder.as_mut(), &query, top_k).await?;
            println!("{}", serde_json::to_string(&hits)?);
        }
        "bench" => {
            ensure!(
                args.len() == 5,
                "bench benötigt Laufweg, Eval-Verzeichnis und Ausgabedatei"
            );
            bench::run(
                &pool,
                embedder.as_mut(),
                backend,
                model_load_ms,
                &PathBuf::from(&args[3]),
                &PathBuf::from(&args[4]),
            )
            .await?;
        }
        _ => bail!("Unbekanntes Dense-Subcommand"),
    }
    pool.close().await;
    Ok(true)
}

fn default_backend() -> &'static str {
    if cfg!(feature = "dense-fastembed") {
        "fastembed"
    } else {
        "candle"
    }
}

fn query_from_stdin() -> Result<String> {
    let mut bytes = Vec::new();
    std::io::stdin().take(4097).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4096, "Dense-Frage ist zu lang");
    let query = String::from_utf8(bytes).context("Frage ist nicht UTF-8")?;
    ensure!(!query.trim().is_empty(), "Dense-Frage ist leer");
    Ok(query.trim().to_string())
}
