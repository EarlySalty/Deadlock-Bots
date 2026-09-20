//! Read-only deployment probe. Prints only capability booleans, never credentials.
use anyhow::{ensure, Result};

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("Produktionsprüfung fehlgeschlagen; keine Verbindungsdetails ausgegeben.");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let pool = dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?;
    let vector: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname='vector')")
            .fetch_one(&pool)
            .await?;
    let schema: bool = sqlx::query_scalar("SELECT COALESCE(has_schema_privilege(current_user, to_regnamespace('knowledge'), 'USAGE'), false)")
        .fetch_one(&pool).await?;
    println!(
        "{}",
        serde_json::json!({"database_connected":true,"vector_extension":vector,"knowledge_schema_usage":schema})
    );
    ensure!(vector && schema, "Vektorschema fehlt");
    for (table, privileges) in [
        ("knowledge.index_generations", "SELECT,INSERT,UPDATE,DELETE"),
        ("knowledge.chunk_embeddings", "SELECT,INSERT,UPDATE,DELETE"),
        ("knowledge.active_index", "SELECT,UPDATE"),
    ] {
        for privilege in privileges.split(',') {
            let allowed: bool = sqlx::query_scalar(
                "SELECT COALESCE(has_table_privilege(current_user, to_regclass($1), $2), false)",
            )
            .bind(table)
            .bind(privilege)
            .fetch_one(&pool)
            .await?;
            println!(
                "{}",
                serde_json::json!({"table":table,"privilege":privilege,"allowed":allowed})
            );
            ensure!(allowed, "Tabellenrecht fehlt");
        }
    }
    let sequence: bool = sqlx::query_scalar("SELECT COALESCE(has_sequence_privilege(current_user,to_regclass('knowledge.index_generations_index_generation_seq'),'USAGE'), false)")
        .fetch_one(&pool).await?;
    println!("{}", serde_json::json!({"sequence_usage":sequence}));
    ensure!(sequence, "Sequenzrecht fehlt");
    Ok(())
}
