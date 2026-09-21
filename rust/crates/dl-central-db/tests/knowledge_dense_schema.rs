use std::path::Path;

fn migration() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/2026091802_knowledge_dense.sql"),
    )
    .expect("Dense-Migration fehlt")
}

#[test]
fn dense_schema_hat_vector_metadaten_und_generationen() {
    let sql = migration();
    for expected in [
        "CREATE EXTENSION IF NOT EXISTS vector",
        "knowledge.index_generations",
        "knowledge.chunk_embeddings",
        "embedding vector(384)",
        "chunk_id",
        "content_hash",
        "stand",
        "quelle",
        "doc_path",
        "model_fingerprint",
        "PRIMARY KEY (index_generation, chunk_id)",
    ] {
        assert!(sql.contains(expected), "Schemafeld fehlt: {expected}");
    }
}

#[test]
fn active_generation_ist_ein_einzelner_ready_verweis() {
    let sql = migration();
    for expected in [
        "knowledge.active_index",
        "singleton boolean PRIMARY KEY CHECK (singleton)",
        "CHECK (state = 'ready')",
        "FOREIGN KEY (index_generation, state)",
        "REFERENCES knowledge.index_generations (index_generation, state)",
    ] {
        assert!(
            sql.contains(expected),
            "Aktivierungsvertrag fehlt: {expected}"
        );
    }
}

#[test]
fn dienstrolle_bekommt_dml_aber_keine_ddl_rechte() {
    let sql = migration();
    assert!(sql.contains("dl_knowledge_dml"));
    assert!(sql.contains("NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE"));
    assert!(sql.contains("GRANT SELECT, INSERT, UPDATE, DELETE"));
    assert!(sql.contains("REVOKE ALL ON SCHEMA knowledge FROM PUBLIC"));
    assert!(!sql.contains("GRANT ALL"));
    assert!(!sql.contains("GRANT CREATE"));
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn dense_migration_ist_anwendbar_und_dienstrolle_ohne_create(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::test_pool().await?;
    let dims: String = sqlx::query_scalar(
        "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
         WHERE attrelid = 'knowledge.chunk_embeddings'::regclass AND attname = 'embedding'",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(dims, "vector(384)");
    let dml: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('dl_knowledge_dml', 'knowledge.chunk_embeddings', 'INSERT')",
    )
    .fetch_one(db.pool())
    .await?;
    let create: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege('dl_knowledge_dml', 'knowledge', 'CREATE')",
    )
    .fetch_one(db.pool())
    .await?;
    assert!(dml);
    assert!(!create);
    let slots: i64 = sqlx::query_scalar("SELECT count(*) FROM knowledge.active_index")
        .fetch_one(db.pool())
        .await?;
    assert_eq!(slots, 1);
    Ok(())
}
