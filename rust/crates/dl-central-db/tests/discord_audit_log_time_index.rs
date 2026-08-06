#![cfg(feature = "testing")]

use dl_central_db::testing::test_pool;

// Der Zeitindex ist die einzige Bremse gegen einen Seq Scan auf einer Tabelle,
// die INSERT-only und ohne Retention waechst. Faellt er aus den Migrationen,
// merkt das sonst erst die Produktion.
#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn discord_audit_log_hat_zeitindex_auf_guild_und_zeitpunkt() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    let index_definition: Option<String> = sqlx::query_scalar(
        "SELECT indexdef
           FROM pg_indexes
          WHERE schemaname = 'core'
            AND tablename = 'discord_audit_log'
            AND indexname = 'idx_discord_audit_log_guild_time'",
    )
    .fetch_optional(pool)
    .await
    .expect("read audit log indexes");

    let index_definition =
        index_definition.expect("idx_discord_audit_log_guild_time muss migriert sein");
    assert!(
        index_definition.contains("guild_id") && index_definition.contains("occurred_at"),
        "Zeitindex muss guild_id und occurred_at fuehren: {index_definition}"
    );
}
