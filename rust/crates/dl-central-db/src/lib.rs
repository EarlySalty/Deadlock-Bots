pub mod core_users;
pub mod pool;

pub use core_users::{get_user, upsert_user, CoreUser};
pub use pool::{connect_pool, dsn_from_env};

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
}
