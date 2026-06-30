use std::process::ExitCode;

use dl_central_db::{connect_pool, dsn_from_env};

#[tokio::main]
async fn main() -> ExitCode {
    eprintln!("dl-central-migrate: wende zentrale DB-Migrationen an …");

    let result = async {
        let dsn = dsn_from_env()?;
        let pool = connect_pool(&dsn).await?;
        sqlx::migrate!("../../crates/dl-central-db/migrations")
            .run(&pool)
            .await?;
        Ok::<(), dl_central_db::CentralDbError>(())
    }
    .await;

    match result {
        Ok(()) => {
            eprintln!("dl-central-migrate: Migrationen erfolgreich angewendet.");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("dl-central-migrate: Migration fehlgeschlagen: {err}");
            ExitCode::FAILURE
        }
    }
}
