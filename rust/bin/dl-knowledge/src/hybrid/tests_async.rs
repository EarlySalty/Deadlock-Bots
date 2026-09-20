use super::*;
use std::time::{Duration, Instant};

struct TestEmbedder;

impl Embedder for TestEmbedder {
    fn fingerprint(&self) -> &str {
        "abababababababababababababababababababababababababababababababab"
    }
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0; crate::dense::DIMENSIONS];
                vector[usize::from(!text.contains("Steam"))] = 1.0;
                vector
            })
            .collect())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_timeout_haelt_kapazitaet_bis_inferenz_beendet() -> Result<()> {
    let worker = Arc::new(worker::Worker::new(100));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let running = worker.clone();
    let task = tokio::spawn(async move {
        running
            .run(move |_| {
                let _ = started_tx.send(());
                release_rx.recv()?;
                Ok(())
            })
            .await
    });
    started_rx.await?;
    assert!(task.await?.is_err());
    assert!(worker.run(|_| Ok(())).await.is_err());
    release_tx.send(())?;
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        if worker.run(|_| Ok(())).await.is_ok() {
            break;
        }
        assert!(Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_client_abbruch_startet_keine_zweite_inferenz() -> Result<()> {
    let worker = Arc::new(worker::Worker::new(2000));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let running = worker.clone();
    let task = tokio::spawn(async move {
        running
            .run(move |_| {
                let _ = started_tx.send(());
                release_rx.recv()?;
                Ok(())
            })
            .await
    });
    started_rx.await?;
    task.abort();
    assert!(task
        .await
        .expect_err("Testtask wurde abgebrochen")
        .is_cancelled());
    assert!(worker.run(|_| Ok(())).await.is_err());
    release_tx.send(())?;
    Ok(())
}

#[tokio::test]
async fn worker_fehler_wird_nicht_als_ergebnis_ausgegeben() {
    let worker = worker::Worker::new(1000);
    assert!(worker
        .run::<(), _>(|_| anyhow::bail!("Testfehler"))
        .await
        .is_err());
    assert_eq!(
        worker
            .run(|_| Ok(42))
            .await
            .expect("Gültiger Testwert erwartet"),
        42
    );
}

#[tokio::test]
async fn dense_filter_und_korpusabgleich_greifen_vor_limit() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let knowledge = knowledge();
    let mut config = Config::default();
    config.metadata.stand_min = Some("2026-09-18".into());
    let catalog = rank::Catalog::new(&knowledge, &config)?;
    let mut embedder = TestEmbedder;
    let built =
        crate::dense::store::build_generation(db.pool(), &mut embedder, &catalog.records).await?;
    crate::dense::store::activate(db.pool(), built.index_generation, None).await?;
    let vector = embedder.embed(&["Steam".into()])?.remove(0);
    assert_eq!(
        search::dense(db.pool(), embedder.fingerprint(), &vector, &catalog, 1)
            .await?
            .1,
        vec![1]
    );
    let mut changed = knowledge.clone();
    changed.chunks[1].text = "Neuer Text".into();
    let fresh = rank::Catalog::new(&changed, &config)?;
    assert!(
        search::dense(db.pool(), embedder.fingerprint(), &vector, &fresh, 100)
            .await
            .is_err()
    );
    changed.chunks[1] = knowledge.chunks[1].clone();
    changed.chunks[1].quelle = "Neue Quelle".into();
    let fresh = rank::Catalog::new(&changed, &config)?;
    assert!(
        search::dense(db.pool(), embedder.fingerprint(), &vector, &fresh, 100)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn dense_fehlender_index_und_fremdes_modell_sind_fehler() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let catalog = rank::Catalog::new(&knowledge(), &Config::default())?;
    let mut embedder = TestEmbedder;
    let vector = embedder.embed(&["Steam".into()])?.remove(0);
    assert!(
        search::dense(db.pool(), embedder.fingerprint(), &vector, &catalog, 6)
            .await
            .is_err()
    );
    let built =
        crate::dense::store::build_generation(db.pool(), &mut embedder, &catalog.records).await?;
    crate::dense::store::activate(db.pool(), built.index_generation, None).await?;
    assert!(
        search::dense(db.pool(), "fremdes-modell", &vector, &catalog, 6)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn dense_gefaelschter_text_wird_nie_gerendert() -> Result<()> {
    let db = crate::test_database::pool().await?;
    let catalog = rank::Catalog::new(&knowledge(), &Config::default())?;
    let mut embedder = TestEmbedder;
    let built =
        crate::dense::store::build_generation(db.pool(), &mut embedder, &catalog.records).await?;
    crate::dense::store::activate(db.pool(), built.index_generation, None).await?;
    sqlx::query("UPDATE knowledge.chunk_embeddings SET chunk_text = 'Fremder Inhalt' WHERE index_generation = $1")
        .bind(built.index_generation).execute(db.pool()).await?;
    let vector = embedder.embed(&["Steam".into()])?.remove(0);
    assert!(
        search::dense(db.pool(), embedder.fingerprint(), &vector, &catalog, 6)
            .await
            .is_err()
    );
    Ok(())
}

#[path = "tests_http.rs"]
mod http;
