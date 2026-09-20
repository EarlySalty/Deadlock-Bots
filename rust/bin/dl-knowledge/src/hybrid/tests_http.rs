use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct SelectionGenerator(AtomicUsize);

#[async_trait::async_trait]
impl crate::TextGenerator for SelectionGenerator {
    async fn generate_text(&self, _: crate::GenerateRequest) -> Option<String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Some(r#"{"candidate_ids":["P1"]}"#.into())
    }
}

struct InvalidReranker;

impl rerank::Reranker for InvalidReranker {
    fn fingerprint(&self) -> &str {
        "test-reranker"
    }
    fn scores(&mut self, _: &str, documents: &[String]) -> Result<Vec<f32>> {
        Ok(vec![f32::NAN; documents.len()])
    }
}

fn state(
    pool: PgPool,
    config: Config,
    generator: Arc<SelectionGenerator>,
    reranker: Option<Box<dyn rerank::Reranker>>,
) -> crate::AppState {
    crate::AppState {
        reload_gate: Default::default(),
        hybrid: Some(Arc::new(Runtime {
            index_model: ("fixture".into(), "/unused/model".into(), Default::default()),
            worker: worker::Worker::new(config.timeout_ms),
            config,
            models: Arc::new(Mutex::new(Models {
                embedder: Box::new(TestEmbedder),
                reranker,
            })),
            catalog: Arc::new(Mutex::new(None)),
            pool,
        })),
        docs_path: PathBuf::from("/unused/public"),
        knowledge: Arc::new(RwLock::new(knowledge())),
        generator: Some(generator),
    }
}

async fn indexed(pool: &PgPool) -> Result<()> {
    let catalog = rank::Catalog::new(&knowledge(), &Config::default())?;
    let mut embedder = TestEmbedder;
    let built =
        crate::dense::store::build_generation(pool, &mut embedder, &catalog.records).await?;
    crate::dense::store::activate(pool, built.index_generation, None).await
}

#[tokio::test]
async fn hybrid_bewahrt_antwortvertrag_rendering_und_grounding() -> Result<()> {
    let _serial = crate::tests::ASK_SERIAL.lock().await;
    let db = crate::test_database::pool().await?;
    indexed(db.pool()).await?;
    let generator = Arc::new(SelectionGenerator(AtomicUsize::new(0)));
    let config = Config {
        rerank: false,
        ..Config::default()
    };
    let app = state(db.pool().clone(), config, generator.clone(), None);
    let response = crate::ask(
        axum::extract::State(app.clone()),
        axum::Json(crate::AskRequest {
            question: "Steam verknüpfen".into(),
        }),
    )
    .await
    .0;
    assert!(response.answerable);
    assert!(response
        .answer
        .as_ref()
        .expect("Gültiger Testwert erwartet")
        .contains("Steam"));
    let json = serde_json::to_value(&response)?;
    assert_eq!(
        json.as_object().expect("Gültiger Testwert erwartet").len(),
        3
    );
    assert!(
        json.get("answerable").is_some()
            && json.get("answer").is_some()
            && json.get("sources").is_some()
    );
    assert_eq!(generator.0.load(Ordering::SeqCst), 1);
    let response = crate::ask(
        axum::extract::State(app),
        axum::Json(crate::AskRequest {
            question: "qwertzxyz unfindbar".into(),
        }),
    )
    .await
    .0;
    assert!(!response.answerable && response.answer.is_none() && response.sources.is_empty());
    assert_eq!(generator.0.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn hybrid_filter_darf_nicht_auf_ungefiltertes_bm25_zurueckfallen() -> Result<()> {
    let _serial = crate::tests::ASK_SERIAL.lock().await;
    let db = crate::test_database::pool().await?;
    indexed(db.pool()).await?;
    let generator = Arc::new(SelectionGenerator(AtomicUsize::new(0)));
    let mut config = Config {
        rerank: false,
        ..Config::default()
    };
    config.metadata.quellen = vec!["nicht-freigegeben".into()];
    let app = state(db.pool().clone(), config, generator.clone(), None);
    let response = crate::ask(
        axum::extract::State(app),
        axum::Json(crate::AskRequest {
            question: "Steam verknüpfen".into(),
        }),
    )
    .await
    .0;
    assert!(!response.answerable && response.sources.is_empty());
    assert_eq!(generator.0.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn hybrid_rerankerfehler_bleibt_fail_closed() -> Result<()> {
    let _serial = crate::tests::ASK_SERIAL.lock().await;
    let db = crate::test_database::pool().await?;
    indexed(db.pool()).await?;
    let generator = Arc::new(SelectionGenerator(AtomicUsize::new(0)));
    let app = state(
        db.pool().clone(),
        Config::default(),
        generator.clone(),
        Some(Box::new(InvalidReranker)),
    );
    let response = crate::ask(
        axum::extract::State(app),
        axum::Json(crate::AskRequest {
            question: "Steam verknüpfen".into(),
        }),
    )
    .await
    .0;
    assert!(!response.answerable && response.answer.is_none() && response.sources.is_empty());
    assert_eq!(generator.0.load(Ordering::SeqCst), 0);
    Ok(())
}
