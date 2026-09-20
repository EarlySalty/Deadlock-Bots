use anyhow::{ensure, Context, Result};
use pgvector::Vector;
use sqlx::{PgPool, Row};

use super::rank::Catalog;

pub async fn dense(
    pool: &PgPool,
    fingerprint: &str,
    vector: &[f32],
    catalog: &Catalog,
    top_k: usize,
) -> Result<(i64, Vec<usize>)> {
    crate::dense::validate_embedding(vector)?;
    ensure!((1..=100).contains(&top_k), "Ungültiges Dense-Limit");
    let active = sqlx::query(
        "SELECT g.index_generation, g.model_fingerprint
         FROM knowledge.active_index a JOIN knowledge.index_generations g
         ON g.index_generation = a.index_generation WHERE a.singleton AND g.state = 'ready'",
    )
    .fetch_optional(pool)
    .await?
    .context("Kein aktiver Dense-Index")?;
    let generation: i64 = active.try_get("index_generation")?;
    ensure!(
        active.try_get::<String, _>("model_fingerprint")? == fingerprint,
        "Query-Modell passt nicht zur aktiven Generation"
    );
    // Never silently run dense on a previous partial corpus. The full current
    // catalogue is checked before metadata filters and before nearest neighbours.
    let all_ids = catalog
        .records
        .iter()
        .map(|r| r.chunk_id.clone())
        .collect::<Vec<_>>();
    let all_hashes = catalog
        .records
        .iter()
        .map(|r| r.content_hash.clone())
        .collect::<Vec<_>>();
    let all_dates = catalog
        .records
        .iter()
        .map(|r| r.stand.clone())
        .collect::<Vec<_>>();
    let all_sources = catalog
        .records
        .iter()
        .map(|r| r.quelle.clone())
        .collect::<Vec<_>>();
    let matching: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge.chunk_embeddings e
         JOIN unnest($2::text[], $3::text[], $4::text[], $5::text[]) AS current(chunk_id, content_hash, stand, quelle)
         USING (chunk_id, content_hash, stand, quelle) WHERE e.index_generation = $1",
    )
    .bind(generation)
    .bind(&all_ids)
    .bind(&all_hashes)
    .bind(&all_dates)
    .bind(&all_sources)
    .fetch_one(pool)
    .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge.chunk_embeddings WHERE index_generation = $1",
    )
    .bind(generation)
    .fetch_one(pool)
    .await?;
    ensure!(
        matching == i64::try_from(catalog.records.len())? && total == matching,
        "Dense-Index gehört nicht vollständig zum aktiven öffentlichen Korpus"
    );
    let eligible = catalog
        .records
        .iter()
        .zip(&catalog.allowed)
        .filter(|(_, allowed)| **allowed)
        .map(|(record, _)| record)
        .collect::<Vec<_>>();
    let ids = eligible
        .iter()
        .map(|record| record.chunk_id.clone())
        .collect::<Vec<_>>();
    let hashes = eligible
        .iter()
        .map(|record| record.content_hash.clone())
        .collect::<Vec<_>>();
    let dates = eligible
        .iter()
        .map(|record| record.stand.clone())
        .collect::<Vec<_>>();
    let sources = eligible
        .iter()
        .map(|record| record.quelle.clone())
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        "SELECT e.chunk_id, e.doc_path, e.title, e.section, e.chunk_text, e.stand, e.quelle,
                1.0 - (e.embedding <=> $2) AS score
         FROM knowledge.chunk_embeddings e
         JOIN unnest($3::text[], $4::text[], $5::text[], $6::text[])
              AS allowed(chunk_id, content_hash, stand, quelle)
         USING (chunk_id, content_hash, stand, quelle)
         WHERE e.index_generation = $1
         ORDER BY e.embedding <=> $2, e.chunk_id LIMIT $7",
    )
    .bind(generation)
    .bind(Vector::from(vector.to_vec()))
    .bind(ids)
    .bind(hashes)
    .bind(dates)
    .bind(sources)
    .bind(i64::try_from(top_k)?)
    .fetch_all(pool)
    .await?;
    let mut indices = Vec::new();
    for row in rows {
        let id: String = row.try_get("chunk_id")?;
        let index = *catalog
            .indices
            .get(&id)
            .context("Dense-ID fehlt im öffentlichen Korpus")?;
        let record = &catalog.records[index];
        ensure!(
            row.try_get::<String, _>("doc_path")? == record.doc_path
                && row.try_get::<String, _>("title")? == record.title
                && row.try_get::<String, _>("section")? == record.section
                && row.try_get::<String, _>("chunk_text")? == record.text
                && row.try_get::<String, _>("stand")? == record.stand
                && row.try_get::<String, _>("quelle")? == record.quelle
                && row.try_get::<f64, _>("score")?.is_finite(),
            "Dense-Treffer passt nicht zum geladenen Korpus"
        );
        indices.push(index);
    }
    Ok((generation, indices))
}
