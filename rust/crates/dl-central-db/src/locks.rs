use sqlx::{Postgres, Transaction};

const RAW_EVENT_RETENTION_ERASURE_LOCK_KEY: i64 = 0x5241_5745_5241_5345;

/// Serialisiert die Raw-Event-Retention mit Löschpfaden, die dieselben
/// nutzerbezogenen Rohzeilen verändern.
pub async fn lock_raw_event_retention_erasure(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(RAW_EVENT_RETENTION_ERASURE_LOCK_KEY)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

/// Serialisiert nutzerbezogene Writes mit Privacy-Erasure bis zum Ende der
/// aufrufenden Transaktion.
pub async fn lock_user_privacy(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(user_id ^ i64::MIN)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

/// Sperrt den User und prueft den Privacy-Grabstein unter demselben Lock.
pub async fn lock_user_privacy_and_is_opted_out(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    lock_user_privacy(tx, user_id).await?;
    sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM core.user_privacy
              WHERE user_id = $1 AND (opted_out = TRUE OR deleted_at IS NOT NULL)
         )",
    )
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn user_privacy_lock_does_not_read_privacy_table(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::testing::test_pool().await?;
        let mut tx = db.pool().begin().await?;
        sqlx::query("ALTER TABLE core.user_privacy RENAME TO user_privacy_hidden")
            .execute(&mut *tx)
            .await?;

        tokio::time::timeout(Duration::from_secs(1), lock_user_privacy(&mut tx, 42)).await??;
        tx.rollback().await?;
        Ok(())
    }

    #[tokio::test]
    async fn user_privacy_lock_is_reentrant() -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::testing::test_pool().await?;
        let mut tx = db.pool().begin().await?;

        lock_user_privacy(&mut tx, 42).await?;
        tokio::time::timeout(Duration::from_secs(1), lock_user_privacy(&mut tx, 42)).await??;
        tx.rollback().await?;
        Ok(())
    }

    #[tokio::test]
    async fn user_privacy_lock_reports_opted_out_tombstone(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::testing::test_pool().await?;
        sqlx::query(
            "INSERT INTO core.user_privacy(user_id, opted_out, updated_at) \
             VALUES(42, TRUE, now())",
        )
        .execute(db.pool())
        .await?;
        let mut tx = db.pool().begin().await?;

        assert!(lock_user_privacy_and_is_opted_out(&mut tx, 42).await?);
        tx.rollback().await?;
        Ok(())
    }
}
