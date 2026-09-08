use std::{
    ops::Deref,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};

use crate::CentralDbError;

const TEST_DSN_ENV_VARS: [&str; 2] = ["CENTRAL_TEST_DSN", "DATABASE_URL"];
const PROD_DB_NAMES: &[&str] = &["deadlock"];
static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TestDb {
    pool: Option<PgPool>,
    admin_options: PgConnectOptions,
    db_name: String,
}

pub async fn set_scrim_runtime_turniere(pool: &PgPool) -> Result<(), CentralDbError> {
    for (expected_epoch, mode) in [(0_i64, "draining"), (1_i64, "turniere")] {
        let applied: bool = sqlx::query_scalar(
            "SELECT applied
               FROM scrim.transition_runtime_control(
                    $1, $2, 'turniere', '42', 'Test',
                    'runtime:test', 'runtime:test', '{}'::jsonb
               )",
        )
        .bind(expected_epoch)
        .bind(mode)
        .fetch_one(pool)
        .await?;
        if !applied {
            return Err(CentralDbError::TestHarness(format!(
                "Scrim-Runtime-Testwechsel auf {mode} wurde abgelehnt"
            )));
        }
    }
    Ok(())
}

pub async fn set_scrim_runtime_inconsistent(pool: &PgPool) -> Result<(), CentralDbError> {
    sqlx::query("ALTER VIEW scrim.runtime_control RENAME TO runtime_control_consistent")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE VIEW scrim.runtime_control AS
         SELECT control_key,
                'turniere'::text AS mode,
                epoch,
                'dl-bots'::text AS operational_writer,
                actor_type,
                actor_pseudonym,
                actor_source,
                request_id,
                correlation_id,
                decision_data,
                created_at,
                updated_at
           FROM scrim.runtime_control_consistent",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_scrim_runtime_missing(pool: &PgPool) -> Result<(), CentralDbError> {
    sqlx::query("ALTER VIEW scrim.runtime_control RENAME TO runtime_control_consistent")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE VIEW scrim.runtime_control AS
         SELECT *
           FROM scrim.runtime_control_consistent
          WHERE false",
    )
    .execute(pool)
    .await?;
    Ok(())
}

impl TestDb {
    pub fn pool(&self) -> &PgPool {
        self.pool
            .as_ref()
            .expect("TestDb pool must be available until drop")
    }
}

impl Deref for TestDb {
    type Target = PgPool;

    fn deref(&self) -> &Self::Target {
        self.pool()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let Some(pool) = self.pool.take() else {
            return;
        };

        let admin_options = self.admin_options.clone();
        let db_name = std::mem::take(&mut self.db_name);
        if db_name.is_empty() {
            return;
        }

        let thread = std::thread::Builder::new()
            .name("dl-central-testdb-drop".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        eprintln!("failed to build TestDb teardown runtime for {db_name}: {err}");
                        return;
                    }
                };

                runtime.block_on(async move {
                    if tokio::time::timeout(Duration::from_secs(5), pool.close())
                        .await
                        .is_err()
                    {
                        eprintln!(
                            "timed out closing TestDb pool for {db_name}; forcing database teardown"
                        );
                    }

                    let admin_pool = match PgPoolOptions::new()
                        .max_connections(1)
                        .connect_with(admin_options)
                        .await
                    {
                        Ok(pool) => pool,
                        Err(err) => {
                            eprintln!(
                                "failed to open fresh admin connection for TestDb teardown of {db_name}: {err}"
                            );
                            return;
                        }
                    };

                    if let Err(err) = drop_database(&admin_pool, &db_name).await {
                        eprintln!("failed to drop TestDb database {db_name}: {err}");
                    }

                    admin_pool.close().await;
                });
            });

        match thread {
            Ok(handle) => {
                if handle.join().is_err() {
                    eprintln!("TestDb teardown thread panicked");
                }
            }
            Err(err) => eprintln!("failed to spawn TestDb teardown thread: {err}"),
        }
    }
}

pub async fn test_pool() -> Result<TestDb, CentralDbError> {
    let dsn = test_dsn_from_env()?;
    let admin_options = PgConnectOptions::from_str(&dsn)?;
    test_pool_with_options(admin_options).await
}

/// Explizite Testverbindung, etwa Unix-Socket mit Peer-Authentifizierung.
/// Erstellt dieselbe isolierte Wegwerf-Datenbank mit garantiertem Drop-Cleanup.
pub async fn test_pool_with_options(
    admin_options: PgConnectOptions,
) -> Result<TestDb, CentralDbError> {
    if admin_options
        .get_database()
        .is_some_and(|database| PROD_DB_NAMES.contains(&database))
    {
        return Err(CentralDbError::TestHarness(
            "Testlauf mit einer Produktionsdatenbank als Ausgangspunkt verweigert".into(),
        ));
    }
    let db_name = unique_db_name();

    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(admin_options.clone())
        .await?;

    create_database(&admin_pool, &db_name).await?;

    let test_options = admin_options.clone().database(&db_name);
    let pool_result = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(test_options)
        .await;

    let pool = match pool_result {
        Ok(pool) => pool,
        Err(err) => {
            let _ = drop_database(&admin_pool, &db_name).await;
            admin_pool.close().await;
            return Err(err.into());
        }
    };

    if let Err(err) = sqlx::migrate!("./migrations").run(&pool).await {
        pool.close().await;
        let _ = drop_database(&admin_pool, &db_name).await;
        admin_pool.close().await;
        return Err(err.into());
    }

    admin_pool.close().await;

    Ok(TestDb {
        pool: Some(pool),
        admin_options,
        db_name,
    })
}

fn test_dsn_from_env() -> Result<String, CentralDbError> {
    let candidates = TEST_DSN_ENV_VARS.map(|name| (name, std::env::var(name).ok()));
    pick_test_dsn(&candidates)
}

fn pick_test_dsn(candidates: &[(&str, Option<String>)]) -> Result<String, CentralDbError> {
    let dsn = candidates
        .iter()
        .find_map(|(_, dsn)| dsn.as_deref())
        .ok_or_else(|| {
            CentralDbError::TestHarness(
                "CENTRAL_TEST_DSN muss gesetzt sein; rust/scripts/central_test_db.sh richtet die Testinstanz ein"
                    .to_owned(),
            )
        })?;
    let options = PgConnectOptions::from_str(dsn)?;

    if let Some(database) = options
        .get_database()
        .filter(|database| PROD_DB_NAMES.contains(database))
    {
        return Err(CentralDbError::TestHarness(format!(
            "Testlauf gegen die Produktionsdatenbank '{database}' verweigert; CENTRAL_TEST_DSN auf eine Testinstanz setzen"
        )));
    }

    Ok(dsn.to_owned())
}

fn unique_db_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);

    format!(
        "dlcentral_test_{}_{}_{}",
        std::process::id(),
        nanos,
        counter
    )
}

async fn create_database(pool: &PgPool, db_name: &str) -> Result<(), CentralDbError> {
    // The identifier is generated by this harness from a fixed prefix plus a
    // process/time suffix; still validate before interpolating into SQL.
    let ident = quoted_ident(db_name)?;
    sqlx::query(&format!("CREATE DATABASE {ident}"))
        .execute(pool)
        .await?;

    Ok(())
}

async fn drop_database(pool: &PgPool, db_name: &str) -> Result<(), CentralDbError> {
    let ident = quoted_ident(db_name)?;
    let terminate_result = terminate_database_sessions(pool, db_name).await;
    let drop_result = sqlx::query(&format!("DROP DATABASE IF EXISTS {ident} WITH (FORCE)"))
        .execute(pool)
        .await;

    match (terminate_result, drop_result) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(err), Ok(_)) => Err(err),
        (Ok(()), Err(err)) => Err(err.into()),
        (Err(terminate_err), Err(drop_err)) => Err(CentralDbError::TestHarness(format!(
            "Test-Datenbank-Teardown unvollstaendig: Sessions terminieren fehlgeschlagen: {terminate_err}; DROP DATABASE fehlgeschlagen: {drop_err}"
        ))),
    }
}

async fn terminate_database_sessions(pool: &PgPool, db_name: &str) -> Result<(), CentralDbError> {
    sqlx::query(
        "SELECT pg_terminate_backend(pid) \
         FROM pg_stat_activity \
         WHERE datname = $1 AND pid <> pg_backend_pid()",
    )
    .bind(db_name)
    .execute(pool)
    .await?;

    Ok(())
}

fn quoted_ident(ident: &str) -> Result<String, CentralDbError> {
    let valid = !ident.is_empty()
        && ident
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');

    if valid {
        Ok(format!(r#""{ident}""#))
    } else {
        Err(CentralDbError::TestHarness(format!(
            "ungueltiger Test-Datenbankname: {ident}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::pick_test_dsn;

    #[test]
    fn picks_central_test_dsn_for_a_test_database() {
        let candidates = [(
            "CENTRAL_TEST_DSN",
            Some("postgres://localhost/deadlock_test".to_owned()),
        )];

        let dsn = pick_test_dsn(&candidates).expect("test database DSN should be accepted");

        assert_eq!(dsn, "postgres://localhost/deadlock_test");
    }

    #[test]
    fn missing_dsn_names_required_variable_and_setup_script() {
        let err = pick_test_dsn(&[("CENTRAL_TEST_DSN", None), ("DATABASE_URL", None)])
            .expect_err("missing test DSN should be rejected");
        let message = err.to_string();

        assert!(message.contains("CENTRAL_TEST_DSN"));
        assert!(message.contains("rust/scripts/central_test_db.sh"));
    }

    #[test]
    fn rejects_production_database_from_every_candidate() {
        for variable in ["CENTRAL_TEST_DSN", "DATABASE_URL"] {
            let err =
                pick_test_dsn(&[(variable, Some("postgres://localhost/deadlock".to_owned()))])
                    .expect_err("production database DSN should be rejected");
            let message = err.to_string();

            assert!(message.contains("deadlock"), "{variable}: {message}");
            assert!(
                message.contains("CENTRAL_TEST_DSN"),
                "{variable}: {message}"
            );
        }
    }

    #[test]
    fn accepts_dsn_without_explicit_database_name() {
        let candidates = [("CENTRAL_TEST_DSN", Some("postgres://localhost".to_owned()))];

        assert!(pick_test_dsn(&candidates).is_ok());
    }
}
