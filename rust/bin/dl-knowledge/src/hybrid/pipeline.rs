use std::time::Instant;

use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sqlx::PgPool;

use crate::KnowledgeBase;

use super::config::Config;
use super::rank::{self, Catalog, Scored};
use super::{search, Models};

#[derive(Debug, Default, Serialize)]
pub struct Timing {
    pub prepare_ms: f64,
    pub bm25_ms: f64,
    pub dense_ms: f64,
    pub fusion_ms: f64,
    pub rerank_ms: f64,
    pub without_rerank_ms: f64,
    pub total_ms: f64,
}

pub struct Retrieved {
    pub generation: Option<i64>,
    pub bm25: Vec<usize>,
    pub dense: Vec<usize>,
    pub fused: Vec<Scored>,
    pub reranked: Vec<Scored>,
    pub timing: Timing,
}

impl Retrieved {
    pub fn chunks(&self, knowledge: &KnowledgeBase, limit: usize) -> Vec<(crate::Chunk, f64)> {
        self.reranked
            .iter()
            .take(limit)
            .map(|hit| (knowledge.chunks[hit.index].clone(), hit.score))
            .collect()
    }
}

pub async fn run(
    pool: &PgPool,
    models: &mut Models,
    knowledge: &KnowledgeBase,
    catalog: &Catalog,
    question: &str,
    config: &Config,
    deadline: Instant,
) -> Result<Retrieved> {
    let start = Instant::now();
    ensure!(
        !question.trim().is_empty() && question.chars().count() <= 4000,
        "Hybrid-Frage ist leer oder zu lang"
    );
    config.validate()?;
    let mut timing = Timing {
        prepare_ms: elapsed(start),
        ..Timing::default()
    };
    let stage = Instant::now();
    let bm25 = catalog.bm25(knowledge, question, config);
    timing.bm25_ms = elapsed(stage);
    ensure!(Instant::now() < deadline, "Hybrid-Zeitbudget überschritten");
    let stage = Instant::now();
    let (generation, dense) = if config.dense_weight > 0.0 {
        let fingerprint = models.embedder.fingerprint().to_string();
        let vectors = models.embedder.embed_queries(&[question.to_string()])?;
        ensure!(
            vectors.len() == 1 && models.embedder.fingerprint() == fingerprint,
            "Ungültiges Query-Embedding oder Modellwechsel"
        );
        let dense_fetch_k = if config.dense_relevance_filter {
            config.dense_k.saturating_mul(3).min(100)
        } else {
            config.dense_k
        };
        let (generation, hits) = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            search::dense(pool, &fingerprint, &vectors[0], catalog, dense_fetch_k),
        )
        .await
        .context("Dense-Suche hat ihr Zeitbudget überschritten")??;
        let hits = if config.dense_relevance_filter {
            rank::relevant_dense(&hits, knowledge, question, config.dense_k)
        } else {
            hits
        };
        (Some(generation), hits)
    } else {
        (None, Vec::new())
    };
    timing.dense_ms = elapsed(stage);
    let stage = Instant::now();
    let fused = rank::fuse(&bm25, &dense, catalog, config);
    timing.fusion_ms = elapsed(stage);
    timing.without_rerank_ms = elapsed(start);
    ensure!(Instant::now() < deadline, "Hybrid-Zeitbudget überschritten");
    let stage = Instant::now();
    let reranked = if config.rerank && !fused.is_empty() {
        let reranker = models
            .reranker
            .as_mut()
            .context("Reranker fehlt trotz aktivem Schalter")?;
        let fingerprint = reranker.fingerprint().to_string();
        let documents = fused
            .iter()
            .map(|hit| catalog.records[hit.index].embedding_text())
            .collect::<Vec<_>>();
        let logits = reranker.scores(question, &documents)?;
        ensure!(
            reranker.fingerprint() == fingerprint,
            "Reranker wurde während der Suche gewechselt"
        );
        rank::apply_rerank(&fused, &logits, catalog, config)?
    } else {
        fused.clone()
    };
    timing.rerank_ms = elapsed(stage);
    timing.total_ms = elapsed(start);
    ensure!(Instant::now() < deadline, "Hybrid-Zeitbudget überschritten");
    Ok(Retrieved {
        generation,
        bm25,
        dense,
        fused,
        reranked,
        timing,
    })
}

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
