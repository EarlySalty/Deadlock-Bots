use dl_brain::api_ingest::ingest_deadlock_api;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let dsn = std::env::var("DEADLOCK_CENTRAL_DSN")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&dsn)
        .await?;
    let summary = ingest_deadlock_api(&pool).await?;
    println!("{}", serde_json::to_string(&summary)?);
    Ok(())
}
