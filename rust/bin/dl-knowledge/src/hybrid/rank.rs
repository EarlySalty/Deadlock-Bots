use std::collections::{HashMap, HashSet};

use anyhow::{ensure, Result};

use crate::dense::{from_chunks, DenseChunk};
use crate::KnowledgeBase;

use super::config::{day, Config};

pub struct Catalog {
    pub records: Vec<DenseChunk>,
    pub indices: HashMap<String, usize>,
    pub allowed: Vec<bool>,
    pub newest: Option<i64>,
}

impl Catalog {
    pub fn new(knowledge: &KnowledgeBase, config: &Config) -> Result<Self> {
        let records = from_chunks(&knowledge.chunks)?;
        let indices = records
            .iter()
            .enumerate()
            .map(|(i, record)| (record.chunk_id.clone(), i))
            .collect();
        let allowed = records
            .iter()
            .map(|record| config.metadata.accepts(&record.stand, &record.quelle))
            .collect();
        let newest = records.iter().filter_map(|record| day(&record.stand)).max();
        Ok(Self {
            records,
            indices,
            allowed,
            newest,
        })
    }

    pub fn bm25(&self, knowledge: &KnowledgeBase, query: &str, config: &Config) -> Vec<usize> {
        if config.bm25_weight == 0.0 {
            return Vec::new();
        }
        knowledge
            .index
            .search(&crate::expand_query(query), knowledge.chunks.len())
            .into_iter()
            .map(|(i, _)| i)
            .filter(|i| self.allowed[*i])
            .take(config.bm25_k)
            .collect()
    }

    pub fn multiplier(&self, index: usize, config: &Config) -> f64 {
        let record = &self.records[index];
        config
            .metadata
            .multiplier(&record.stand, &record.quelle, self.newest)
    }
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub index: usize,
    pub score: f64,
}

pub fn fuse(bm25: &[usize], dense: &[usize], catalog: &Catalog, config: &Config) -> Vec<Scored> {
    let mut scores = HashMap::<usize, f64>::new();
    for (ranked, weight) in [(bm25, config.bm25_weight), (dense, config.dense_weight)] {
        if weight == 0.0 {
            continue;
        }
        let mut seen = HashSet::new();
        let mut rank = 0;
        for index in ranked {
            if *index >= catalog.records.len() || !catalog.allowed[*index] || !seen.insert(*index) {
                continue;
            }
            rank += 1;
            *scores.entry(*index).or_default() += weight / (config.rrf_k + rank as f64);
        }
    }
    if config.source_consensus_weight > 0.0 {
        let mut bm25_sources = HashMap::<&str, (usize, usize)>::new();
        for (rank, index) in bm25.iter().copied().enumerate() {
            if index < catalog.records.len() && catalog.allowed[index] {
                bm25_sources
                    .entry(catalog.records[index].doc_path.as_str())
                    .or_insert((rank + 1, index));
            }
        }
        let mut dense_sources = HashMap::<&str, usize>::new();
        for (rank, index) in dense.iter().copied().enumerate() {
            if index < catalog.records.len() && catalog.allowed[index] {
                dense_sources
                    .entry(catalog.records[index].doc_path.as_str())
                    .or_insert(rank + 1);
            }
        }
        for (source, (bm25_rank, representative)) in bm25_sources {
            let Some(dense_rank) = dense_sources.get(source) else {
                continue;
            };
            let bonus = config.source_consensus_weight
                * (config.bm25_weight / (config.rrf_k + bm25_rank as f64)
                    + config.dense_weight / (config.rrf_k + *dense_rank as f64));
            *scores.entry(representative).or_default() += bonus;
        }
    }
    let mut ranked = scores
        .into_iter()
        .map(|(index, score)| Scored {
            index,
            score: score * catalog.multiplier(index, config),
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.score.total_cmp(&a.score).then_with(|| {
            catalog.records[a.index]
                .chunk_id
                .cmp(&catalog.records[b.index].chunk_id)
        })
    });
    ranked.truncate(config.fusion_k);
    ranked
}

pub fn apply_rerank(
    ranked: &[Scored],
    logits: &[f32],
    catalog: &Catalog,
    config: &Config,
) -> Result<Vec<Scored>> {
    ensure!(
        ranked.len() == logits.len() && logits.iter().all(|score| score.is_finite()),
        "Ungültige Reranker-Scores"
    );
    let mut rescored = ranked
        .iter()
        .zip(logits)
        .enumerate()
        .map(|(position, (hit, score))| {
            let probability = 1.0 / (1.0 + (-f64::from(*score)).exp());
            (
                position,
                Scored {
                    index: hit.index,
                    score: probability * catalog.multiplier(hit.index, config),
                },
            )
        })
        .collect::<Vec<_>>();
    rescored.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    Ok(rescored.into_iter().map(|(_, hit)| hit).collect())
}
