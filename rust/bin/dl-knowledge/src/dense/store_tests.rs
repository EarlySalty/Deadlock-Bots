use anyhow::{bail, Result};
use sqlx::postgres::PgPoolOptions;

use super::*;
use crate::dense::{sha256, DIMENSIONS};

struct MockEmbedder {
    fingerprint: String,
    computed: usize,
    fail: bool,
}

impl MockEmbedder {
    fn new(version: &str) -> Self {
        Self {
            fingerprint: sha256(version.as_bytes()),
            computed: 0,
            fail: false,
        }
    }
}

impl Embedder for MockEmbedder {
    fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if self.fail {
            bail!("Absichtlicher Embedding-Fehler");
        }
        self.computed += texts.len();
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0; DIMENSIONS];
                vector[usize::from(!text.to_lowercase().contains("steam"))] = 1.0;
                vector
            })
            .collect())
    }
}

fn record(id: &str, text: &str) -> DenseChunk {
    let mut chunk = DenseChunk {
        chunk_id: sha256(id.as_bytes()),
        content_hash: String::new(),
        doc_path: format!("{id}.html"),
        title: "Hilfe".to_string(),
        section: id.to_string(),
        text: text.to_string(),
        stand: "2026-09-18".to_string(),
        quelle: "Öffentliche Doku".to_string(),
    };
    chunk.content_hash = sha256(chunk.embedding_text().as_bytes());
    chunk
}

fn records() -> Vec<DenseChunk> {
    vec![
        record("steam", "Steam-Konto verknüpfen."),
        record("discord", "Discord-Rollen auswählen."),
    ]
}

async fn active(pool: &PgPool) -> Result<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT index_generation FROM knowledge.active_index WHERE singleton")
            .fetch_one(pool)
            .await?,
    )
}

#[tokio::test]
async fn neuer_index_bleibt_bis_zur_expliziten_aktivierung_green() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let mut embedder = MockEmbedder::new("v1");
    let built = build_generation(db.pool(), &mut embedder, &records()).await?;
    assert_eq!((built.chunks, built.computed, built.reused), (2, 2, 0));
    assert_eq!(active(db.pool()).await?, None);
    assert!(dense_search(db.pool(), &mut embedder, "Steam", 1)
        .await?
        .is_empty());
    activate(db.pool(), built.index_generation, None).await?;
    assert_eq!(active(db.pool()).await?, Some(built.index_generation));
    Ok(())
}

#[tokio::test]
async fn inkrementell_kopiert_hash_aktualisiert_metadaten_und_entfernt_alte_chunks() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let mut embedder = MockEmbedder::new("v1");
    let old = build_generation(db.pool(), &mut embedder, &records()).await?;
    activate(db.pool(), old.index_generation, None).await?;
    let mut changed = records();
    changed[0].stand = "2026-09-19".to_string();
    changed[0].quelle = "Neue Quelle".to_string();
    changed.pop();
    let new = build_generation(db.pool(), &mut embedder, &changed).await?;
    assert_eq!(
        (new.chunks, new.computed, new.reused, embedder.computed),
        (1, 0, 1, 2)
    );
    assert_eq!(active(db.pool()).await?, Some(old.index_generation));
    activate(db.pool(), new.index_generation, Some(old.index_generation)).await?;
    let hits = dense_search(db.pool(), &mut embedder, "Steam", 10).await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].stand, "2026-09-19");
    assert_eq!(hits[0].quelle, "Neue Quelle");
    assert_eq!(hits[0].index_generation, new.index_generation);
    let changed = vec![record("steam", "Steam mit geändertem Inhalt.")];
    let newest = build_generation(db.pool(), &mut embedder, &changed).await?;
    assert_eq!((newest.computed, newest.reused), (1, 0));
    Ok(())
}

#[tokio::test]
async fn fehlgeschlagener_lauf_rollt_zurueck_und_behaelt_blue() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let mut embedder = MockEmbedder::new("v1");
    let old = build_generation(db.pool(), &mut embedder, &records()).await?;
    activate(db.pool(), old.index_generation, None).await?;
    embedder.fail = true;
    assert!(
        build_generation(db.pool(), &mut embedder, &[record("neu", "Neuer Inhalt")])
            .await
            .is_err()
    );
    assert_eq!(active(db.pool()).await?, Some(old.index_generation));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM knowledge.index_generations")
        .fetch_one(db.pool())
        .await?;
    assert_eq!(count, 1);
    Ok(())
}

#[tokio::test]
async fn cosine_suche_nutzt_nur_aktive_generation_und_passendes_modell() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let mut embedder = MockEmbedder::new("v1");
    let old = build_generation(db.pool(), &mut embedder, &records()).await?;
    activate(db.pool(), old.index_generation, None).await?;
    let hits = dense_search(db.pool(), &mut embedder, "Steam", 2).await?;
    assert_eq!(hits[0].doc_path, "steam.html");
    assert!((hits[0].score - 1.0).abs() < 1e-6);
    assert!(hits[0].score > hits[1].score);
    let mut other = MockEmbedder::new("v2");
    assert!(dense_search(db.pool(), &mut other, "Steam", 1)
        .await
        .is_err());
    let new = build_generation(db.pool(), &mut other, &records()).await?;
    assert_eq!(new.computed, 2);
    assert_eq!(active(db.pool()).await?, Some(old.index_generation));
    assert!(dense_search(db.pool(), &mut embedder, "", 1).await.is_err());
    assert!(dense_search(db.pool(), &mut embedder, "Steam", 0)
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn aktivierung_verweigert_unfertigen_index_und_veraltetes_compare_and_swap() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let mut embedder = MockEmbedder::new("v1");
    let old = build_generation(db.pool(), &mut embedder, &records()).await?;
    activate(db.pool(), old.index_generation, None).await?;
    let new = build_generation(db.pool(), &mut embedder, &records()).await?;
    assert!(activate(db.pool(), new.index_generation, None)
        .await
        .is_err());
    let unfinished: i64 = sqlx::query_scalar("INSERT INTO knowledge.index_generations (model_fingerprint) VALUES ($1) RETURNING index_generation")
        .bind(embedder.fingerprint()).fetch_one(db.pool()).await?;
    assert!(activate(db.pool(), unfinished, Some(old.index_generation))
        .await
        .is_err());
    assert_eq!(active(db.pool()).await?, Some(old.index_generation));
    activate(db.pool(), new.index_generation, Some(old.index_generation)).await?;
    activate(db.pool(), old.index_generation, Some(new.index_generation)).await?;
    assert_eq!(active(db.pool()).await?, Some(old.index_generation));
    Ok(())
}

#[tokio::test]
async fn index_und_suche_funktionieren_mit_reiner_dml_rolle() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE dl_knowledge_dml")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with((*db.pool().connect_options()).clone())
        .await?;
    let mut embedder = MockEmbedder::new("v1");
    let built = build_generation(&pool, &mut embedder, &records()).await?;
    activate(&pool, built.index_generation, None).await?;
    assert_eq!(
        dense_search(&pool, &mut embedder, "Steam", 1).await?.len(),
        1
    );
    assert!(
        sqlx::query("CREATE TABLE knowledge.nicht_erlaubt (id integer)")
            .execute(&pool)
            .await
            .is_err()
    );
    pool.close().await;
    Ok(())
}
