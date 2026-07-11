pub mod core_users;
pub mod kv;
mod locks;
pub mod pool;
#[cfg(feature = "testing")]
pub mod testing;

pub use core_users::{get_user, upsert_user, CoreUser};
pub use kv::{delete as delete_kv, get as get_kv, set as set_kv};
pub use locks::lock_raw_event_retention_erasure;
pub use pool::{connect_pool, dsn_from_env};
#[cfg(feature = "testing")]
pub use testing::{test_pool, TestDb};

#[derive(Debug, thiserror::Error)]
pub enum CentralDbError {
    #[error("Umgebungsvariable {name} konnte nicht gelesen werden")]
    EnvVar {
        name: &'static str,
        source: std::env::VarError,
    },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("Test-Harness-Setup fehlgeschlagen: {0}")]
    TestHarness(String),
}
