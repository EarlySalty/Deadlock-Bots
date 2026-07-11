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
