// Shared test-only connection configuration for isolated central database fixtures.
pub async fn database() -> dl_central_db::TestDb {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-database.json");
    match std::fs::read(path) {
        Ok(bytes) => {
            let config: serde_json::Value =
                serde_json::from_slice(&bytes).expect("valid local test-database.json");
            let options = config["database_url"]
                .as_str()
                .expect("database_url string")
                .parse::<sqlx::postgres::PgConnectOptions>()
                .expect("valid local test database options");
            dl_central_db::testing::test_pool_with_options(options).await
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            dl_central_db::testing::test_pool().await
        }
        Err(_) => panic!("cannot read local test-database.json"),
    }
    .expect("isolated database")
}
