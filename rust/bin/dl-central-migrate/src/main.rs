use std::process::ExitCode;

use dl_central_db::{connect_pool, dsn_from_env};

const CANONICAL_SCRIM_1602_SHA384: &str =
    "423022abe243dbe00f81ac78fa7b159d842b5257cd38d67abf1fc4b432c00a471645a5fcb60d9fcfd301c74d625ddd78";
const TRANSIENT_SCRIM_1602_SHA384: &str =
    "d9a559ac1702de6542a4a3d86a41267e9e6952be135200f103f739359b6d71fa5d1465928b7321b715d3a570044c186a";

fn is_transient_scrim_checksum(checksum: &str) -> bool {
    checksum == TRANSIENT_SCRIM_1602_SHA384
}

async fn reconcile_transient_scrim_1602_checksum(pool: &sqlx::PgPool) -> Result<bool, sqlx::Error> {
    let migrations_table_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    if !migrations_table_exists {
        return Ok(false);
    }

    let checksum: Option<String> = sqlx::query_scalar(
        "SELECT encode(checksum, 'hex')
           FROM _sqlx_migrations
          WHERE version = 2026071602 AND success",
    )
    .fetch_optional(pool)
    .await?;
    if !checksum.as_deref().is_some_and(is_transient_scrim_checksum) {
        return Ok(false);
    }

    let result = sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = decode($1, 'hex')
          WHERE version = 2026071602
            AND success
            AND encode(checksum, 'hex') = $2",
    )
    .bind(CANONICAL_SCRIM_1602_SHA384)
    .bind(TRANSIENT_SCRIM_1602_SHA384)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

#[tokio::main]
async fn main() -> ExitCode {
    eprintln!("dl-central-migrate: wende zentrale DB-Migrationen an …");

    let result = async {
        let dsn = dsn_from_env()?;
        let pool = connect_pool(&dsn).await?;
        if reconcile_transient_scrim_1602_checksum(&pool).await? {
            eprintln!(
                "dl-central-migrate: kurzzeitig veroeffentlichte Scrim-Migrationspruefsumme abgeglichen."
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_die_bekannte_kurzzeitig_veroeffentlichte_pruefsumme_wird_abgeglichen() {
        assert!(is_transient_scrim_checksum(TRANSIENT_SCRIM_1602_SHA384));
        assert!(!is_transient_scrim_checksum(CANONICAL_SCRIM_1602_SHA384));
        assert!(!is_transient_scrim_checksum("unbekannt"));
    }

    #[tokio::test]
    #[ignore = "braucht die Wegwerf-DB aus central_test_db.sh"]
    async fn bekannte_kurzzeit_pruefsumme_wird_vor_sqlx_lauf_reconciled() {
        let dsn = dsn_from_env().expect("throwaway DSN");
        let pool = connect_pool(&dsn).await.expect("connect throwaway DB");
        sqlx::query(
            "UPDATE _sqlx_migrations
                SET checksum = decode($1, 'hex')
              WHERE version = 2026071602 AND success",
        )
        .bind(TRANSIENT_SCRIM_1602_SHA384)
        .execute(&pool)
        .await
        .expect("simulate transient published checksum");

        assert!(reconcile_transient_scrim_1602_checksum(&pool)
            .await
            .expect("reconcile checksum"));
        let checksum: String = sqlx::query_scalar(
            "SELECT encode(checksum, 'hex')
               FROM _sqlx_migrations
              WHERE version = 2026071602 AND success",
        )
        .fetch_one(&pool)
        .await
        .expect("read reconciled checksum");
        assert_eq!(checksum, CANONICAL_SCRIM_1602_SHA384);

        sqlx::migrate!("../../crates/dl-central-db/migrations")
            .run(&pool)
            .await
            .expect("migrator accepts reconciled history");
    }
}
