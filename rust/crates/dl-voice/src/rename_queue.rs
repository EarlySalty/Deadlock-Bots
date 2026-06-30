//! Zentrale, rate-limit-bewusste Channel-Rename-Queue (Port `cogs/rename_manager.py`).
//!
//! Mehrere Voice-Module benennen denselben Lane-/Status-Channel um; Discord
//! limitiert Channel-Renames hart (≈2/10min). Statt direkt umzubenennen reihen
//! die Glue-Impls ihre Wünsche hier ein (last-wins pro Channel); EIN Worker
//! drainiert die Queue FIFO mit ≥360s Mindestabstand pro Channel. Persistent in
//! `rename_requests`; bei Crash werden hängende `PROCESSING`-Zeilen beim Start
//! wieder auf `PENDING` gesetzt.
//!
//! Die Queue ist ein Prozess-Singleton (`OnceLock`) — der Worker ist per Design
//! genau einer pro Prozess. Ist sie nicht initialisiert (z. B. in Tests),
//! benennt [`enqueue_or_direct`] direkt um (altes Verhalten).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use dl_discord::DiscordAdapter;
use serde_json::json;
use serenity::all::ChannelId;
use sqlx::{PgPool, Postgres, Transaction};
use std::fmt;

use crate::db::{i64_to_u64, u64_to_i64, VoiceDbResult};

const QUEUE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const ERROR_BACKOFF: Duration = Duration::from_secs(10);
const RENAME_THROTTLE: Duration = Duration::from_secs(360);
const MAX_RETRIES: i32 = 5;
const RENAME_REQUEST_ID_LOCK_KEY: i64 = -7_010_030_001;

static QUEUE: OnceLock<RenameQueue> = OnceLock::new();

fn worker_id() -> i64 {
    i64::from(std::process::id())
}

/// Initialisiert die Prozess-globale Queue (idempotent). Einmalig beim Start,
/// bevor die Voice-Subscriber laufen.
pub fn init(pool: PgPool) {
    let _ = QUEUE.get_or_init(|| RenameQueue { pool });
}

fn global() -> Option<&'static RenameQueue> {
    QUEUE.get()
}

/// Reiht eine Umbenennung ein, wenn die globale Queue initialisiert ist; sonst
/// direkter Edit (Fallback). Aufruf aus den Glue-Rename-Impls.
pub async fn enqueue_or_direct(
    adapter: &DiscordAdapter,
    channel_id: u64,
    name: &str,
    reason: &str,
) -> Result<(), String> {
    if let Some(q) = global() {
        return q
            .enqueue(channel_id, name, reason)
            .await
            .map_err(|e| e.to_string());
    }
    adapter
        .http
        .edit_channel(
            ChannelId::new(channel_id),
            &json!({ "name": name }),
            Some(reason),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Die Queue — hält nur den DB-Handle.
#[derive(Clone)]
pub struct RenameQueue {
    pool: PgPool,
}

impl RenameQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Zentrale Migrationen besitzen das DDL; App-seitiges Bootstrap ist no-op.
    pub async fn ensure_schema(&self) -> VoiceDbResult<()> {
        Ok(())
    }

    /// Reiht eine Umbenennung ein — last-wins pro Channel (vorhandene PENDING-
    /// Zeile desselben Channels wird ersetzt). Leerer Name wird ignoriert.
    pub async fn enqueue(
        &self,
        channel_id: u64,
        new_name: &str,
        reason: &str,
    ) -> VoiceDbResult<()> {
        let name = new_name.trim().to_string();
        if name.is_empty() {
            return Ok(());
        }
        let reason = reason.to_string();
        let channel_id = u64_to_i64("rename_requests.channel_id", channel_id)?;
        let mut tx = self.pool.begin().await?;
        lock_rename_request_ids(&mut tx).await?;
        let id = next_rename_request_id(&mut tx).await?;
        sqlx::query!(
            r#"
            DELETE FROM voice.rename_requests
             WHERE channel_id = $1
               AND status = 'PENDING'
            "#,
            channel_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO voice.rename_requests (
                id, channel_id, new_name, reason, status, retry_count, assigned_worker_id
            )
            VALUES ($1, $2, $3, $4, 'PENDING', 0, 0)
            "#,
            id,
            channel_id,
            name,
            reason,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Führt die eigentliche Discord-Umbenennung aus (vom Worker genutzt).
#[async_trait::async_trait]
pub trait RenameExec: Send + Sync {
    /// Aktueller Channel-Name oder `None` (Channel weg/nicht im Cache).
    async fn current_name(&self, channel_id: u64) -> Option<String>;
    /// Benennt den Channel um. `Err` = Fehler (wird ggf. erneut versucht).
    async fn edit_name(&self, channel_id: u64, name: &str, reason: &str)
        -> Result<(), RenameError>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenameError {
    RateLimited { retry_after_seconds: f64 },
    Other(String),
}

impl RenameError {
    pub fn rate_limited(retry_after_seconds: f64) -> Self {
        Self::RateLimited {
            retry_after_seconds,
        }
    }

    pub fn retry_after_seconds(&self) -> Option<f64> {
        match self {
            Self::RateLimited {
                retry_after_seconds,
            } => Some(*retry_after_seconds),
            Self::Other(_) => None,
        }
    }
}

impl fmt::Display for RenameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited {
                retry_after_seconds,
            } => write!(f, "HTTP 429 (retry_after={retry_after_seconds})"),
            Self::Other(err) => f.write_str(err),
        }
    }
}

impl From<String> for RenameError {
    fn from(value: String) -> Self {
        Self::Other(value)
    }
}

impl From<&str> for RenameError {
    fn from(value: &str) -> Self {
        Self::Other(value.to_string())
    }
}

struct Claimed {
    id: i64,
    channel_id: u64,
    new_name: String,
    reason: String,
    retry_count: i32,
}

/// Startet den Rename-Worker (genau EINER pro Prozess).
pub fn spawn_worker(pool: PgPool, exec: Arc<dyn RenameExec>) {
    tokio::spawn(async move {
        let q = RenameQueue { pool };
        if let Err(e) = q.ensure_schema().await {
            tracing::error!(%e, "rename-worker: Schema fehlgeschlagen");
            return;
        }
        // Crash-Recovery: hängende PROCESSING-Zeilen zurück auf PENDING.
        let _ = sqlx::query!(
            r#"
            UPDATE voice.rename_requests
               SET status = 'PENDING',
                   assigned_worker_id = 0,
                   processed_at = NULL
             WHERE status = 'PROCESSING'
            "#
        )
        .execute(&q.pool)
        .await;

        let mut last_attempt: HashMap<u64, Instant> = HashMap::new();
        loop {
            tokio::time::sleep(QUEUE_CHECK_INTERVAL).await;
            let claimed = match claim_next(&q.pool).await {
                Ok(Some(c)) => c,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(%e, "rename-worker: claim fehlgeschlagen");
                    tokio::time::sleep(ERROR_BACKOFF).await;
                    continue;
                }
            };

            // Pro-Channel-Mindestabstand: noch im Fenster → zurück in die Queue.
            if let Some(t) = last_attempt.get(&claimed.channel_id) {
                let elapsed = t.elapsed();
                if elapsed < RENAME_THROTTLE {
                    let last_error = format!(
                        "Channel throttle active ({:.1}s remaining)",
                        (RENAME_THROTTLE - elapsed).as_secs_f64()
                    );
                    let _ = set_pending(&q.pool, claimed.id, Some(&last_error), false).await;
                    let remaining = (RENAME_THROTTLE - elapsed)
                        .min(Duration::from_secs(5))
                        .max(Duration::from_millis(500));
                    tokio::time::sleep(remaining).await;
                    continue;
                }
            }

            match exec.current_name(claimed.channel_id).await {
                None => {
                    let _ = set_failed(&q.pool, claimed.id, "Channel nicht gefunden").await;
                    continue;
                }
                Some(cur) if cur == claimed.new_name => {
                    let _ = set_done(&q.pool, claimed.id).await;
                    continue;
                }
                Some(_) => {}
            }

            match exec
                .edit_name(claimed.channel_id, &claimed.new_name, &claimed.reason)
                .await
            {
                Ok(()) => {
                    last_attempt.insert(claimed.channel_id, Instant::now());
                    let _ = set_done(&q.pool, claimed.id).await;
                }
                Err(e) => {
                    let err_text = e.to_string();
                    if let Some(retry_after) = e.retry_after_seconds() {
                        last_attempt.insert(claimed.channel_id, Instant::now());
                        let _ = set_pending(&q.pool, claimed.id, Some(&err_text), true).await;
                        tokio::time::sleep(Duration::from_secs_f64(retry_after.max(0.0))).await;
                    } else if claimed.retry_count + 1 >= MAX_RETRIES {
                        let _ = set_failed(&q.pool, claimed.id, &err_text).await;
                    } else {
                        let _ = set_pending(&q.pool, claimed.id, Some(&err_text), true).await;
                    }
                }
            }
        }
    });
}

async fn claim_next(pool: &PgPool) -> VoiceDbResult<Option<Claimed>> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query!(
        r#"
        SELECT id,
               channel_id,
               new_name,
               reason,
               COALESCE(retry_count, 0) AS "retry_count!"
          FROM voice.rename_requests
         WHERE status = 'PENDING'
         ORDER BY created_at ASC, id ASC
         LIMIT 1
         FOR UPDATE SKIP LOCKED
        "#
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    let updated = sqlx::query!(
        r#"
        UPDATE voice.rename_requests
           SET status = 'PROCESSING',
               assigned_worker_id = $2,
               processed_at = NOW()
         WHERE id = $1
           AND status = 'PENDING'
        "#,
        row.id,
        worker_id(),
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if updated.rows_affected() == 1 {
        Ok(Some(Claimed {
            id: row.id,
            channel_id: i64_to_u64("rename_requests.channel_id", row.channel_id)?,
            new_name: row.new_name,
            reason: row.reason.unwrap_or_default(),
            retry_count: row.retry_count,
        }))
    } else {
        Ok(None)
    }
}

async fn set_pending(
    pool: &PgPool,
    id: i64,
    last_error: Option<&str>,
    increment_retry: bool,
) -> VoiceDbResult<()> {
    let last_error = last_error.map(|err| err.chars().take(1000).collect::<String>());
    if increment_retry {
        // Retry → ans Ende der FIFO-Queue (created_at neu).
        sqlx::query!(
            r#"
            UPDATE voice.rename_requests
               SET status = 'PENDING',
                   retry_count = COALESCE(retry_count, 0) + 1,
                   created_at = NOW(),
                   processed_at = NULL,
                   assigned_worker_id = 0,
                   last_error = $2
             WHERE id = $1
            "#,
            id,
            last_error,
        )
        .execute(pool)
        .await?;
    } else {
        sqlx::query!(
            r#"
            UPDATE voice.rename_requests
               SET status = 'PENDING',
                   created_at = NOW(),
                   processed_at = NULL,
                   assigned_worker_id = 0,
                   last_error = $2
             WHERE id = $1
            "#,
            id,
            last_error,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn set_done(pool: &PgPool, id: i64) -> VoiceDbResult<()> {
    sqlx::query!(
        r#"
        UPDATE voice.rename_requests
           SET status = 'DONE',
               processed_at = NOW(),
               assigned_worker_id = $2,
               last_error = NULL
         WHERE id = $1
        "#,
        id,
        worker_id(),
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn set_failed(pool: &PgPool, id: i64, err: &str) -> VoiceDbResult<()> {
    let err: String = err.chars().take(1000).collect();
    sqlx::query!(
        r#"
        UPDATE voice.rename_requests
           SET status = 'FAILED',
               processed_at = NOW(),
               assigned_worker_id = $3,
               last_error = $2
         WHERE id = $1
        "#,
        id,
        err,
        worker_id(),
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn lock_rename_request_ids(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        SELECT 1 AS "locked!"
          FROM pg_advisory_xact_lock($1)
        "#,
        RENAME_REQUEST_ID_LOCK_KEY,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

async fn next_rename_request_id(tx: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT COALESCE(MAX(id), 0) + 1 AS "id!"
          FROM voice.rename_requests
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (dl_central_db::TestDb, RenameQueue) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let q = RenameQueue::new(db.pool().clone());
        q.ensure_schema().await.expect("schema");
        (db, q)
    }

    #[tokio::test]
    async fn enqueue_dedup_last_wins() {
        let (_d, q) = mk().await;
        q.enqueue(100, "alt", "r").await.unwrap();
        q.enqueue(100, "neu", "r").await.unwrap();
        q.enqueue(200, "other", "r").await.unwrap();
        let cnt = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM voice.rename_requests
             WHERE channel_id = 100
               AND status = 'PENDING'
            "#
        )
        .fetch_one(&q.pool)
        .await
        .unwrap();
        let name = sqlx::query_scalar!(
            r#"
            SELECT new_name
              FROM voice.rename_requests
             WHERE channel_id = 100
               AND status = 'PENDING'
            "#
        )
        .fetch_one(&q.pool)
        .await
        .unwrap();
        assert_eq!(cnt, 1, "nur eine PENDING-Zeile pro Channel");
        assert_eq!(name, "neu", "letzter Wunsch gewinnt");
    }

    #[tokio::test]
    async fn enqueue_ignoriert_leeren_namen() {
        let (_d, q) = mk().await;
        q.enqueue(1, "   ", "r").await.unwrap();
        let n = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM voice.rename_requests"#)
            .fetch_one(&q.pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn claim_fifo_und_status_transitionen() {
        let (_d, q) = mk().await;
        q.enqueue(10, "a", "r").await.unwrap();
        q.enqueue(20, "b", "r").await.unwrap();

        let first = claim_next(&q.pool).await.unwrap().expect("claim1");
        assert_eq!(first.channel_id, 10, "FIFO: ältester zuerst");
        let assigned = sqlx::query_scalar!(
            r#"
            SELECT assigned_worker_id AS "assigned_worker_id!"
              FROM voice.rename_requests
             WHERE id = $1
            "#,
            first.id,
        )
        .fetch_one(&q.pool)
        .await
        .unwrap();
        assert_ne!(assigned, 0, "Claim setzt wie Python eine Worker-ID");
        let second = claim_next(&q.pool).await.unwrap().expect("claim2");
        assert_eq!(second.channel_id, 20);
        assert!(
            claim_next(&q.pool).await.unwrap().is_none(),
            "keine PENDING mehr"
        );

        let fid = first.id;
        set_done(&q.pool, fid).await.unwrap();
        let done = sqlx::query!(
            r#"
            SELECT status,
                   assigned_worker_id AS "assigned_worker_id!",
                   last_error
              FROM voice.rename_requests
             WHERE id = $1
            "#,
            fid,
        )
        .fetch_one(&q.pool)
        .await
        .unwrap();
        assert_eq!(done.status, "DONE");
        assert_ne!(done.assigned_worker_id, 0);
        assert_eq!(done.last_error, None);

        // Retry erhöht retry_count und setzt zurück auf PENDING.
        let sid = second.id;
        set_pending(&q.pool, sid, Some("HTTP 500: nope"), true)
            .await
            .unwrap();
        let pending = sqlx::query!(
            r#"
            SELECT status,
                   COALESCE(retry_count, 0) AS "retry_count!",
                   assigned_worker_id AS "assigned_worker_id!",
                   last_error
              FROM voice.rename_requests
             WHERE id = $1
            "#,
            sid,
        )
        .fetch_one(&q.pool)
        .await
        .unwrap();
        assert_eq!(pending.status, "PENDING");
        assert_eq!(pending.retry_count, 1);
        assert_eq!(pending.assigned_worker_id, 0);
        assert_eq!(pending.last_error.as_deref(), Some("HTTP 500: nope"));
    }

    #[test]
    fn rename_error_erkennt_http_429_retry_after() {
        let err = RenameError::rate_limited(2.5);
        assert_eq!(err.retry_after_seconds(), Some(2.5));
        assert_eq!(err.to_string(), "HTTP 429 (retry_after=2.5)");
    }
}
