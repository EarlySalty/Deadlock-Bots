use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::dense::{local, Embedder};
use crate::{Chunk, KnowledgeBase};

mod bench;
pub(super) mod cli;
pub(crate) mod config;
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
    fn load_explicit(settings: &crate::server_config::HybridSettings) -> Result<Self> {
        Ok(Self {
            embedder: local::load_profile(
                "fastembed",
                &settings.embedding_model,
                settings.embedding_profile,
            )?,
            reranker: if settings.options.rerank {
                Some(rerank::load(&settings.reranker_model, &settings.options)?)
            } else {
                None
            },
        })
    }
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
    index_model: (String, PathBuf, local::Profile),
}

pub struct PreparedIndex {
    generation: i64,
    previous: Option<i64>,
}

impl Runtime {
    /// Prepare a complete generation before publishing its corresponding BM25 snapshot.
    pub async fn prepare(&self, knowledge: &KnowledgeBase) -> Result<PreparedIndex> {
        let records = crate::dense::from_chunks(&knowledge.chunks)?;
        let (backend, model_root, profile) = self.index_model.clone();
        let models = self.models.clone();
        let pool = self.pool.clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || -> Result<PreparedIndex> {
            // The indexing encoder is transient and independent of the query
            // model mutex. Readers continue using the complete prior snapshot.
            let mut embedder = local::load_profile(&backend, &model_root, profile)?;
            let expected = models
                .lock()
                .map_err(|_| anyhow::anyhow!("Suchmodellzustand unbrauchbar"))?
                .embedder
                .fingerprint()
                .to_string();
            anyhow::ensure!(
                embedder.fingerprint() == expected,
                "Indexencoder passt nicht zur laufenden Query-Modellversion"
            );
            handle.block_on(async {
                let previous: Option<i64> = sqlx::query_scalar(
                    "SELECT index_generation FROM knowledge.active_index WHERE singleton",
                )
                .fetch_one(&pool)
                .await
                .context("Aktiven Suchindex lesen")?;
                let built =
                    crate::dense::store::build_generation(&pool, embedder.as_mut(), &records)
                        .await
                        .context("Öffentlichen Suchindex vorbereiten")?;
                Ok(PreparedIndex {
                    generation: built.index_generation,
                    previous,
                })
            })
        })
        .await
        .context("Index-Worker verbinden")?
    }

    pub async fn activate(&self, prepared: PreparedIndex) -> Result<()> {
        crate::dense::store::activate(&self.pool, prepared.generation, prepared.previous)
            .await
            .context("Vollständigen Suchindex aktivieren")
    }

    pub async fn from_config(settings: &crate::server_config::HybridSettings) -> Result<Arc<Self>> {
        settings.validate()?;
        let loading = settings.clone();
        let models = tokio::task::spawn_blocking(move || Models::load_explicit(&loading))
            .await
            .context("Lokalen Suchmodell-Worker verbinden")??;
        let pool = if let Some(address) = &settings.database_url {
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(4)
                .connect(address)
                .await
                .context("Lokalen Suchindex verbinden")?
        } else {
            dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?
        };
        Ok(Arc::new(Self {
            worker: worker::Worker::new(settings.options.timeout_ms),
            config: settings.options.clone(),
            models: Arc::new(Mutex::new(models)),
            catalog: Arc::new(Mutex::new(None)),
            pool,
            index_model: (
                "fastembed".into(),
                settings.embedding_model.clone(),
                settings.embedding_profile,
            ),
        }))
    }

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
            index_model: (
                std::env::var("DL_KNOWLEDGE_EMBEDDER").unwrap_or_else(|_| {
                    if cfg!(feature = "dense-fastembed") {
                        "fastembed".into()
                    } else {
                        "candle".into()
                    }
                }),
                model_path("DL_KNOWLEDGE_MODELS", "all-MiniLM-L6-v2")?,
                local::Profile::MiniLm,
            ),
        })))
    }

    pub async fn retrieve(
        &self,
        question: &str,
        knowledge: Arc<RwLock<KnowledgeBase>>,
    ) -> Result<Vec<(Chunk, f64)>> {
        let live = knowledge.read().await;
        self.retrieve_snapshot(question, &live).await
    }

    pub async fn retrieve_snapshot(
        &self,
        question: &str,
        live: &KnowledgeBase,
    ) -> Result<Vec<(Chunk, f64)>> {
        anyhow::ensure!(
            !question.trim().is_empty() && question.chars().count() <= 4000,
            "Hybrid-Frage ist leer oder zu lang"
        );
        let question = question.to_string();
        let config = self.config.clone();
        let models = self.models.clone();
        let catalog_cache = self.catalog.clone();
        let pool = self.pool.clone();
        let handle = tokio::runtime::Handle::current();
        let live = live.clone();
        self.worker
            .run(move |deadline| {
                let (knowledge, catalog) = {
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
                Ok(result.chunks(&knowledge, config.output_k))
            })
            .await
    }
}

#[cfg(test)]
mod tests;
