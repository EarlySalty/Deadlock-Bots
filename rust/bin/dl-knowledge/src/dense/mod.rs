use std::collections::HashMap;
use std::path::{Component, Path};

use anyhow::{ensure, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::Chunk;

mod bench;
pub(super) mod cli;
pub(super) mod local;
pub mod store;

pub const DIMENSIONS: usize = 384;
#[cfg(any(feature = "dense-fastembed", feature = "dense-candle"))]
pub const MAX_TOKENS: usize = 256;

pub trait Embedder: Send {
    fn fingerprint(&self) -> &str;
    /// Encode indexed documents according to the model's passage contract.
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Encode search questions; asymmetric encoders override their query prefix.
    fn embed_queries(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed(texts)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DenseChunk {
    pub chunk_id: String,
    pub content_hash: String,
    pub doc_path: String,
    pub title: String,
    pub section: String,
    pub text: String,
    pub stand: String,
    pub quelle: String,
}

impl DenseChunk {
    pub fn embedding_text(&self) -> String {
        format!("{}\n{}\n{}", self.title, self.section, self.text)
    }
}

pub(super) fn from_chunks(chunks: &[Chunk]) -> Result<Vec<DenseChunk>> {
    let mut occurrences = HashMap::<(&str, &str), usize>::new();
    chunks
        .iter()
        .map(|chunk| {
            let path = Path::new(&chunk.path);
            ensure!(
                path.extension().is_some_and(|ext| ext == "html")
                    && !chunk.path.contains('\\')
                    && path.components().all(|part| matches!(part, Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case("internal"))),
                "Dense akzeptiert nur relative öffentliche HTML-Pfade"
            );
            ensure!(
                !chunk.stand.trim().is_empty()
                    && !chunk.quelle.trim().is_empty()
                    && !chunk.text.trim().is_empty(),
                "Dense-Chunk ohne Pflichtmetadaten oder Text"
            );
            let occurrence = occurrences.entry((&chunk.path, &chunk.section)).or_default();
            let identity = serde_json::to_vec(&(&chunk.path, &chunk.section, *occurrence))?;
            *occurrence += 1;
            let mut record = DenseChunk {
                chunk_id: sha256(&identity),
                content_hash: String::new(),
                doc_path: chunk.path.clone(),
                title: chunk.title.clone(),
                section: chunk.section.clone(),
                text: chunk.text.clone(),
                stand: chunk.stand.clone(),
                quelle: chunk.quelle.clone(),
            };
            record.content_hash = sha256(record.embedding_text().as_bytes());
            Ok(record)
        })
        .collect()
}

pub fn validate_embedding(embedding: &[f32]) -> Result<()> {
    ensure!(
        embedding.len() == DIMENSIONS,
        "Embedding benötigt {DIMENSIONS} Dimensionen"
    );
    ensure!(
        embedding.iter().all(|value| value.is_finite()),
        "Embedding enthält nichtendliche Werte"
    );
    let norm: f64 = embedding
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum();
    ensure!(
        norm > 0.0 && norm.is_finite(),
        "Embedding ohne gültige Norm"
    );
    Ok(())
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests;
