use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{ensure, Context, Result};
use pgvector::Vector;
use serde::Serialize;
use sqlx::{PgPool, Row};

use super::{sha256, validate_embedding, DenseChunk, Embedder};

const INDEX_BUILD_LOCK: i64 = 0x646c_6b6e_6f77;

#[derive(Debug, Serialize)]
pub struct IndexBuild {
    pub index_generation: i64,
    pub chunks: usize,
    pub computed: usize,
    pub reused: usize,
}

#[derive(Debug, Serialize)]
pub struct DenseHit {
    pub index_generation: i64,
    pub chunk_id: String,
    pub doc_path: String,
    pub title: String,
    pub section: String,
    pub text: String,
    pub stand: String,
    pub quelle: String,
    pub score: f64,
}

pub async fn build_generation(
    pool: &PgPool,
    embedder: &mut dyn Embedder,
    chunks: &[DenseChunk],
) -> Result<IndexBuild> {
    ensure!(!chunks.is_empty(), "Leerer Korpus wird nicht indexiert");
    ensure!(
        chunks
            .iter()
            .map(|chunk| &chunk.chunk_id)
            .collect::<HashSet<_>>()
            .len()
            == chunks.len(),
        "Doppelte Chunk-ID"
    );
    for chunk in chunks {
        ensure!(
            chunk.content_hash == sha256(chunk.embedding_text().as_bytes()),
            "Content-Hash passt nicht zur Embedding-Eingabe"
        );
    }
    let fingerprint = embedder.fingerprint().to_string();
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(INDEX_BUILD_LOCK)
        .execute(&mut *tx)
        .await?;
    let generation: i64 = sqlx::query_scalar(
        "INSERT INTO knowledge.index_generations (model_fingerprint) VALUES ($1) RETURNING index_generation",
    ).bind(&fingerprint).fetch_one(&mut *tx).await?;
    let cached = sqlx::query(
        "SELECT DISTINCT ON (e.content_hash) e.content_hash, e.embedding
         FROM knowledge.chunk_embeddings e
         JOIN knowledge.index_generations g USING (index_generation)
         WHERE g.state = 'ready' AND g.model_fingerprint = $1
         ORDER BY e.content_hash, e.index_generation DESC",
    )
    .bind(&fingerprint)
    .fetch_all(&mut *tx)
    .await?;
    let mut cache = HashMap::<String, Vector>::new();
    for row in cached {
        let vector: Vector = row.try_get("embedding")?;
        validate_embedding(&vector.to_vec())?;
        cache.insert(row.try_get("content_hash")?, vector);
    }
    let mut missing = BTreeMap::new();
    for chunk in chunks {
        if !cache.contains_key(&chunk.content_hash) {
            missing
                .entry(chunk.content_hash.clone())
                .or_insert_with(|| chunk.embedding_text());
        }
    }
    let missing: Vec<(String, String)> = missing.into_iter().collect();
    let computed = missing.len();
    for batch in missing.chunks(16) {
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        let embeddings = embedder.embed(&texts)?;
        ensure!(
            embeddings.len() == batch.len(),
            "Embedding-Anzahl passt nicht zur Batchgröße"
        );
        ensure!(
            embedder.fingerprint() == fingerprint,
            "Modell wurde während des Indexierens gewechselt"
        );
        for ((hash, _), embedding) in batch.iter().zip(embeddings) {
            validate_embedding(&embedding)?;
            cache.insert(hash.clone(), Vector::from(embedding));
        }
    }
    for chunk in chunks {
        let vector = cache
            .get(&chunk.content_hash)
            .context("Embedding fehlt nach dem Indexlauf")?;
        sqlx::query(
            "INSERT INTO knowledge.chunk_embeddings
             (index_generation, chunk_id, content_hash, embedding, stand, quelle, doc_path, title, section, chunk_text)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        ).bind(generation).bind(&chunk.chunk_id).bind(&chunk.content_hash).bind(vector)
            .bind(&chunk.stand).bind(&chunk.quelle).bind(&chunk.doc_path).bind(&chunk.title)
            .bind(&chunk.section).bind(&chunk.text).execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE knowledge.index_generations SET state = 'ready', chunk_count = $2, completed_at = now()
         WHERE index_generation = $1",
    ).bind(generation).bind(i64::try_from(chunks.len())?).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(IndexBuild {
        index_generation: generation,
        chunks: chunks.len(),
        computed,
        reused: chunks.len() - computed,
    })
}

pub async fn activate(pool: &PgPool, generation: i64, expected: Option<i64>) -> Result<()> {
    let changed = sqlx::query(
        "UPDATE knowledge.active_index SET index_generation = $1, activated_at = now()
         WHERE singleton AND index_generation IS NOT DISTINCT FROM $2
         AND EXISTS (
             SELECT 1 FROM knowledge.index_generations g
             WHERE g.index_generation = $1 AND g.state = 'ready' AND g.chunk_count > 0
             AND g.chunk_count = (SELECT count(*) FROM knowledge.chunk_embeddings e WHERE e.index_generation = g.index_generation)
         )",
    ).bind(generation).bind(expected).execute(pool).await?.rows_affected();
    ensure!(
        changed == 1,
        "Aktivierung verweigert: Index unvollständig oder aktiver Stand inzwischen geändert"
    );
    Ok(())
}

pub async fn dense_search(
    pool: &PgPool,
    embedder: &mut dyn Embedder,
    query: &str,
    top_k: usize,
) -> Result<Vec<DenseHit>> {
    ensure!(
        !query.trim().is_empty() && query.len() <= 4096,
        "Dense-Frage ist leer oder zu lang"
    );
    ensure!(
        (1..=100).contains(&top_k),
        "Dense top_k muss zwischen 1 und 100 liegen"
    );
    let fingerprint = embedder.fingerprint().to_string();
    let embeddings = embedder.embed(&[query.to_string()])?;
    ensure!(embeddings.len() == 1, "Query-Embedding-Anzahl ist ungültig");
    ensure!(
        fingerprint == embedder.fingerprint(),
        "Modell wurde während der Suche gewechselt"
    );
    search_vector(pool, &fingerprint, &embeddings[0], top_k).await
}

pub async fn search_vector(
    pool: &PgPool,
    fingerprint: &str,
    vector: &[f32],
    top_k: usize,
) -> Result<Vec<DenseHit>> {
    validate_embedding(vector)?;
    ensure!(
        (1..=100).contains(&top_k),
        "Dense top_k muss zwischen 1 und 100 liegen"
    );
    let active = sqlx::query(
        "SELECT g.index_generation, g.model_fingerprint
         FROM knowledge.active_index a
         JOIN knowledge.index_generations g ON g.index_generation = a.index_generation
         WHERE a.singleton AND g.state = 'ready'",
    )
    .fetch_optional(pool)
    .await?;
    let Some(active) = active else {
        return Ok(Vec::new());
    };
    let stored_fingerprint: String = active.try_get("model_fingerprint")?;
    ensure!(
        stored_fingerprint == fingerprint,
        "Query-Modell passt nicht zur aktiven Indexgeneration"
    );
    let generation: i64 = active.try_get("index_generation")?;
    let rows = sqlx::query(
        "SELECT index_generation, chunk_id, doc_path, title, section, chunk_text, stand, quelle,
                1.0 - (embedding <=> $2) AS score
         FROM knowledge.chunk_embeddings
         WHERE index_generation = $1
         ORDER BY embedding <=> $2, chunk_id
         LIMIT $3",
    )
    .bind(generation)
    .bind(Vector::from(vector.to_vec()))
    .bind(i64::try_from(top_k)?)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(DenseHit {
                index_generation: row.try_get("index_generation")?,
                chunk_id: row.try_get("chunk_id")?,
                doc_path: row.try_get("doc_path")?,
                title: row.try_get("title")?,
                section: row.try_get("section")?,
                text: row.try_get("chunk_text")?,
                stand: row.try_get("stand")?,
                quelle: row.try_get("quelle")?,
                score: row.try_get("score")?,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
