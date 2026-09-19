use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::dense::{local, Embedder};
use crate::{Chunk, KnowledgeBase};

mod bench;
pub(super) mod cli;
mod config;
mod pipeline;
mod quality;
mod rank;
mod rerank;
mod search;
mod worker;

use config::Config;
use rank::Catalog;

pub struct Models {
    pub embedder: Box<dyn Embedder>,
    pub reranker: Option<Box<dyn rerank::Reranker>>,
}

impl Models {
    fn load(config: &Config) -> Result<Self> {
        let backend = std::env::var("DL_KNOWLEDGE_EMBEDDER").unwrap_or_else(|_| {
            if cfg!(feature = "dense-fastembed") {
                "fastembed"
            } else {
                "candle"
            }
            .to_string()
        });
        let root = model_path("DL_KNOWLEDGE_MODELS", "all-MiniLM-L6-v2")?;
        Ok(Self {
            embedder: local::load(&backend, &root)?,
            reranker: if config.rerank {
                Some(rerank::load(
                    &model_path("DL_KNOWLEDGE_RERANK_MODELS", "ms-marco-MiniLM-L6-v2")?,
                    config,
                )?)
            } else {
                None
            },
        })
    }
}

fn model_path(variable: &str, name: &str) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(variable) {
        return Ok(PathBuf::from(path));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("HOME fehlt für den Modellpfad")?)
            .join(".local/share/dl-knowledge/models")
            .join(name),
    )
}

type CatalogCache = Arc<Mutex<Option<(u64, Arc<KnowledgeBase>, Arc<Catalog>)>>>;

pub struct Runtime {
    config: Config,
    models: Arc<Mutex<Models>>,
    catalog: CatalogCache,
    pool: PgPool,
    worker: worker::Worker,
}

impl Runtime {
    pub async fn from_env() -> Result<Option<Arc<Self>>> {
        let enabled = std::env::var("DL_KNOWLEDGE_HYBRID").ok();
        let options = std::env::var("DL_KNOWLEDGE_HYBRID_OPTIONS").ok();
        let Some(config) = Config::from_values(enabled.as_deref(), options.as_deref())? else {
            return Ok(None);
        };
        let load_config = config.clone();
        let models = tokio::task::spawn_blocking(move || Models::load(&load_config)).await??;
        let pool = dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?;
        Ok(Some(Arc::new(Self {
            worker: worker::Worker::new(config.timeout_ms),
            config,
            models: Arc::new(Mutex::new(models)),
            catalog: Arc::new(Mutex::new(None)),
            pool,
        })))
    }

    pub async fn retrieve(
        &self,
        question: &str,
        knowledge: Arc<RwLock<KnowledgeBase>>,
    ) -> Result<Vec<(Chunk, f64)>> {
        anyhow::ensure!(
            !question.trim().is_empty() && question.len() <= 4096,
            "Hybrid-Frage ist leer oder zu lang"
        );
        let question = question.to_string();
        let config = self.config.clone();
        let models = self.models.clone();
        let catalog_cache = self.catalog.clone();
        let pool = self.pool.clone();
        let handle = tokio::runtime::Handle::current();
        self.worker
            .run(move |deadline| {
                let (knowledge, catalog) = {
                    let live = knowledge.blocking_read();
                    let mut cache = catalog_cache
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Hybrid-Katalogzustand unbrauchbar"))?;
                    if let Some((_, knowledge, catalog)) = cache
                        .as_ref()
                        .filter(|(generation, _, _)| *generation == live.generation)
                    {
                        (knowledge.clone(), catalog.clone())
                    } else {
                        let snapshot = Arc::new(live.clone());
                        let catalog = Arc::new(Catalog::new(&snapshot, &config)?);
                        *cache = Some((snapshot.generation, snapshot.clone(), catalog.clone()));
                        (snapshot, catalog)
                    }
                };
                let mut models = models
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Hybrid-Modellzustand unbrauchbar"))?;
                let result = handle.block_on(pipeline::run(
                    &pool,
                    &mut models,
                    knowledge.as_ref(),
                    catalog.as_ref(),
                    &question,
                    &config,
                    deadline,
                ))?;
                tracing::debug!(
                    generation = result.generation,
                    bm25_hits = result.bm25.len(),
                    dense_hits = result.dense.len(),
                    fused_hits = result.fused.len(),
                    retrieval_ms = result.timing.total_ms,
                    rerank_ms = result.timing.rerank_ms,
                    "Hybrid-Retrieval"
                );
                Ok(result
                    .reranked
                    .into_iter()
                    .take(config.output_k)
                    .map(|hit| (knowledge.chunks[hit.index].clone(), hit.score))
                    .collect())
            })
            .await
    }
}

#[cfg(test)]
mod tests;
