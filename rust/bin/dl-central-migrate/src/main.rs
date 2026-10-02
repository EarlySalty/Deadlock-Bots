use std::process::ExitCode;

use dl_central_db::{connect_pool, dsn_from_env};

mod peer_config;

const STEAM_CREDENTIAL_VERSIONS: [i64; 2] = [20260930220100, 20260930220400];

fn steam_credentials_migrator(
    applied: &std::collections::BTreeSet<i64>,
) -> sqlx::migrate::Migrator {
    let mut migrator = sqlx::migrate!("../../crates/dl-central-db/migrations");
    migrator.migrations = std::borrow::Cow::Owned(
        migrator
            .iter()
            .filter(|migration| {
                applied.contains(&migration.version)
                    || STEAM_CREDENTIAL_VERSIONS.contains(&migration.version)
            })
            .cloned()
            .collect(),
    );
    migrator
}

const CANONICAL_SCRIM_1602_SHA384: &str =
    "423022abe243dbe00f81ac78fa7b159d842b5257cd38d67abf1fc4b432c00a471645a5fcb60d9fcfd301c74d625ddd78";
const TRANSIENT_SCRIM_1602_SHA384: &str =
    "d9a559ac1702de6542a4a3d86a41267e9e6952be135200f103f739359b6d71fa5d1465928b7321b715d3a570044c186a";

fn is_transient_scrim_checksum(checksum: &str) -> bool {
    checksum == TRANSIENT_SCRIM_1602_SHA384
}

fn validate_steam_history(checksum: Option<&str>) -> anyhow::Result<()> {
    anyhow::ensure!(
        !checksum.is_some_and(is_transient_scrim_checksum),
        "Begrenzte Steam-Migration abgebrochen: vorhandene Scrim-Prüfsumme benötigt eine separate Freigabe. In diesem Modus wird keine fremde Migrationshistorie geändert."
    );
    Ok(())
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
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let steam_only = args.first().is_some_and(|arg| arg == "--steam-credentials-only");
        let connection_args = if steam_only { &args[1..] } else { &args[..] };
        anyhow::ensure!(
            !steam_only || !connection_args.is_empty(),
            "Begrenzte Steam-Migration benötigt eine explizite lokale Peer-Konfiguration."
        );
        let pool = if let Some(options) = peer_config::from_args(connection_args)? {
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await?
        } else {
            let dsn = dsn_from_env()?;
            connect_pool(&dsn).await?
        };
        if steam_only {
            // Dieser Modus gilt für die bestehende zentrale Datenbank. Auch die
            // bekannte alte Scrim-Historie wird hier ausdrücklich nicht repariert.
            let checksum: Option<String> = sqlx::query_scalar(
                "SELECT encode(checksum, 'hex') FROM public._sqlx_migrations WHERE version=2026071602 AND success"
            ).fetch_optional(&pool).await?;
            validate_steam_history(checksum.as_deref())?;
            // SQLx behält Lock, Ledger und Prüfsummenprüfung. Bereits angewandte
            // fremde Versionen bleiben enthalten; offene fremde Versionen nicht.
            let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM public._sqlx_migrations")
                .fetch_all(&pool)
                .await?;
            let migrator = steam_credentials_migrator(&applied.into_iter().collect());
            migrator.run(&pool).await?;
            return Ok::<(), anyhow::Error>(());
        }
        if reconcile_transient_scrim_1602_checksum(&pool).await? {
            eprintln!(
                "dl-central-migrate: kurzzeitig veroeffentlichte Scrim-Migrationspruefsumme abgeglichen."
            );
        }
        sqlx::migrate!("../../crates/dl-central-db/migrations")
            .run(&pool)
            .await?;
        Ok::<(), anyhow::Error>(())
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
    fn steam_mode_rejects_legacy_checksum_without_modifying_foreign_history() {
        assert!(validate_steam_history(Some(TRANSIENT_SCRIM_1602_SHA384)).is_err());
        assert!(validate_steam_history(Some(CANONICAL_SCRIM_1602_SHA384)).is_ok());
        // Unbekannte Prüfsummen bleiben der unveränderten SQLx-Prüfung überlassen.
        assert!(validate_steam_history(Some("unknown")).is_ok());
        assert!(validate_steam_history(None).is_ok());
    }

    #[test]
    fn steam_mode_preserves_history_and_excludes_unrelated_pending_migrations() {
        let applied = std::collections::BTreeSet::from([2026071602, 2026093004]);
        let migrator = steam_credentials_migrator(&applied);
        let selected: std::collections::BTreeSet<_> = migrator.iter().map(|m| m.version).collect();
        assert_eq!(
            selected,
            applied
                .union(&STEAM_CREDENTIAL_VERSIONS.into_iter().collect())
                .copied()
                .collect()
        );
        assert!(!selected.contains(&20260930220000));
        assert!(!migrator.ignore_missing);
        assert!(migrator.locking);
    }

    #[test]
    fn steam_mode_keeps_already_applied_token_hash_migration_for_checksum_validation() {
        let applied = std::collections::BTreeSet::from([20260930220000]);
        let migrator = steam_credentials_migrator(&applied);
        assert!(migrator.iter().any(|m| m.version == 20260930220000));
        assert_eq!(migrator.iter().count(), 3);
    }

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
