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

use dl_db::{Db, DbError};
use dl_discord::DiscordAdapter;
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use serenity::all::ChannelId;

const QUEUE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const ERROR_BACKOFF: Duration = Duration::from_secs(10);
const RENAME_THROTTLE: Duration = Duration::from_secs(360);
const MAX_RETRIES: i64 = 5;

static QUEUE: OnceLock<RenameQueue> = OnceLock::new();

/// Initialisiert die Prozess-globale Queue (idempotent). Einmalig beim Start,
/// bevor die Voice-Subscriber laufen.
pub fn init(db: Db) {
    let _ = QUEUE.get_or_init(|| RenameQueue { db });
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
    db: Db,
}

impl RenameQueue {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Legt die Queue-Tabelle an (idempotent). Schema wie `rename_manager.py`.
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS rename_requests (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       channel_id INTEGER NOT NULL,
                       new_name TEXT NOT NULL,
                       reason TEXT,
                       status TEXT NOT NULL DEFAULT 'PENDING',
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                       processed_at TIMESTAMP,
                       retry_count INTEGER DEFAULT 0,
                       last_error TEXT,
                       assigned_worker_id INTEGER DEFAULT 0
                     );
                     CREATE INDEX IF NOT EXISTS idx_rename_requests_status_created
                       ON rename_requests(status, created_at, id);",
                )?;
                Ok(())
            })
            .await
    }

    /// Reiht eine Umbenennung ein — last-wins pro Channel (vorhandene PENDING-
    /// Zeile desselben Channels wird ersetzt). Leerer Name wird ignoriert.
    pub async fn enqueue(
        &self,
        channel_id: u64,
        new_name: &str,
        reason: &str,
    ) -> Result<(), DbError> {
        let name = new_name.trim().to_string();
        if name.is_empty() {
            return Ok(());
        }
        let reason = reason.to_string();
        let cid = channel_id as i64;
        self.db
            .write(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "DELETE FROM rename_requests WHERE channel_id=?1 AND status='PENDING'",
                    params![cid],
                )?;
                tx.execute(
                    "INSERT INTO rename_requests(channel_id, new_name, reason, status)
                     VALUES (?1, ?2, ?3, 'PENDING')",
                    params![cid, name, reason],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

/// Führt die eigentliche Discord-Umbenennung aus (vom Worker genutzt).
#[async_trait::async_trait]
pub trait RenameExec: Send + Sync {
    /// Aktueller Channel-Name oder `None` (Channel weg/nicht im Cache).
    async fn current_name(&self, channel_id: u64) -> Option<String>;
    /// Benennt den Channel um. `Err` = Fehler (wird ggf. erneut versucht).
    async fn edit_name(&self, channel_id: u64, name: &str, reason: &str) -> Result<(), String>;
}

struct Claimed {
    id: i64,
    channel_id: u64,
    new_name: String,
    reason: String,
    retry_count: i64,
}

/// Startet den Rename-Worker (genau EINER pro Prozess).
pub fn spawn_worker(db: Db, exec: Arc<dyn RenameExec>) {
    tokio::spawn(async move {
        let q = RenameQueue { db };
        if let Err(e) = q.ensure_schema().await {
            tracing::error!(%e, "rename-worker: Schema fehlgeschlagen");
            return;
        }
        // Crash-Recovery: hängende PROCESSING-Zeilen zurück auf PENDING.
        let _ = q
            .db
            .write(|conn| {
                conn.execute(
                    "UPDATE rename_requests SET status='PENDING', assigned_worker_id=0, processed_at=NULL WHERE status='PROCESSING'",
                    [],
                )?;
                Ok(())
            })
            .await;

        let mut last_attempt: HashMap<u64, Instant> = HashMap::new();
        loop {
            tokio::time::sleep(QUEUE_CHECK_INTERVAL).await;
            let claimed = match claim_next(&q.db).await {
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
                    let _ = set_pending(&q.db, claimed.id, false).await;
                    let remaining = (RENAME_THROTTLE - elapsed)
                        .min(Duration::from_secs(5))
                        .max(Duration::from_millis(500));
                    tokio::time::sleep(remaining).await;
                    continue;
                }
            }

            match exec.current_name(claimed.channel_id).await {
                None => {
                    let _ = set_failed(&q.db, claimed.id, "Channel nicht gefunden").await;
                    continue;
                }
                Some(cur) if cur == claimed.new_name => {
                    let _ = set_done(&q.db, claimed.id).await;
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
                    let _ = set_done(&q.db, claimed.id).await;
                }
                Err(e) => {
                    if claimed.retry_count + 1 >= MAX_RETRIES {
                        let _ = set_failed(&q.db, claimed.id, &e).await;
                    } else {
                        let _ = set_pending(&q.db, claimed.id, true).await;
                    }
                }
            }
        }
    });
}

async fn claim_next(db: &Db) -> Result<Option<Claimed>, DbError> {
    db.write(|conn| {
        let tx = conn.transaction()?;
        let row = tx
            .query_row(
                "SELECT id, channel_id, new_name, reason, retry_count FROM rename_requests
                 WHERE status='PENDING' ORDER BY created_at ASC, id ASC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, channel_id, new_name, reason, retry_count)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        let updated = tx.execute(
            "UPDATE rename_requests SET status='PROCESSING', processed_at=CURRENT_TIMESTAMP
             WHERE id=?1 AND status='PENDING'",
            params![id],
        )?;
        tx.commit()?;
        if updated == 1 {
            Ok(Some(Claimed {
                id,
                channel_id: channel_id as u64,
                new_name,
                reason: reason.unwrap_or_default(),
                retry_count,
            }))
        } else {
            Ok(None)
        }
    })
    .await
}

async fn set_pending(db: &Db, id: i64, increment_retry: bool) -> Result<(), DbError> {
    db.write(move |conn| {
        if increment_retry {
            // Retry → ans Ende der FIFO-Queue (created_at neu).
            conn.execute(
                "UPDATE rename_requests SET status='PENDING', retry_count=retry_count+1, created_at=CURRENT_TIMESTAMP, processed_at=NULL WHERE id=?1",
                params![id],
            )?;
        } else {
            // Throttle-Aufschub → FIFO-Position behalten.
            conn.execute(
                "UPDATE rename_requests SET status='PENDING', processed_at=NULL WHERE id=?1",
                params![id],
            )?;
        }
        Ok(())
    })
    .await
}

async fn set_done(db: &Db, id: i64) -> Result<(), DbError> {
    db.write(move |conn| {
        conn.execute(
            "UPDATE rename_requests SET status='DONE', processed_at=CURRENT_TIMESTAMP WHERE id=?1",
            params![id],
        )?;
        Ok(())
    })
    .await
}

async fn set_failed(db: &Db, id: i64, err: &str) -> Result<(), DbError> {
    let err: String = err.chars().take(1000).collect();
    db.write(move |conn| {
        conn.execute(
            "UPDATE rename_requests SET status='FAILED', processed_at=CURRENT_TIMESTAMP, last_error=?2 WHERE id=?1",
            params![id, err],
        )?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (tempfile::TempDir, RenameQueue) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let q = RenameQueue::new(db);
        q.ensure_schema().await.expect("schema");
        (dir, q)
    }

    #[tokio::test]
    async fn enqueue_dedup_last_wins() {
        let (_d, q) = mk().await;
        q.enqueue(100, "alt", "r").await.unwrap();
        q.enqueue(100, "neu", "r").await.unwrap();
        q.enqueue(200, "other", "r").await.unwrap();
        let (cnt, name): (i64, String) = q
            .db
            .read(|c| {
                Ok((
                    c.query_row(
                        "SELECT COUNT(*) FROM rename_requests WHERE channel_id=100 AND status='PENDING'",
                        [],
                        |r| r.get(0),
                    )?,
                    c.query_row(
                        "SELECT new_name FROM rename_requests WHERE channel_id=100 AND status='PENDING'",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(cnt, 1, "nur eine PENDING-Zeile pro Channel");
        assert_eq!(name, "neu", "letzter Wunsch gewinnt");
    }

    #[tokio::test]
    async fn enqueue_ignoriert_leeren_namen() {
        let (_d, q) = mk().await;
        q.enqueue(1, "   ", "r").await.unwrap();
        let n: i64 =
            q.db.read(|c| c.query_row("SELECT COUNT(*) FROM rename_requests", [], |r| r.get(0)))
                .await
                .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn claim_fifo_und_status_transitionen() {
        let (_d, q) = mk().await;
        q.enqueue(10, "a", "r").await.unwrap();
        q.enqueue(20, "b", "r").await.unwrap();

        let first = claim_next(&q.db).await.unwrap().expect("claim1");
        assert_eq!(first.channel_id, 10, "FIFO: ältester zuerst");
        let second = claim_next(&q.db).await.unwrap().expect("claim2");
        assert_eq!(second.channel_id, 20);
        assert!(
            claim_next(&q.db).await.unwrap().is_none(),
            "keine PENDING mehr"
        );

        let fid = first.id;
        set_done(&q.db, fid).await.unwrap();
        let status: String =
            q.db.read(move |c| {
                c.query_row(
                    "SELECT status FROM rename_requests WHERE id=?1",
                    params![fid],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(status, "DONE");

        // Retry erhöht retry_count und setzt zurück auf PENDING.
        let sid = second.id;
        set_pending(&q.db, sid, true).await.unwrap();
        let (st, rc): (String, i64) =
            q.db.read(move |c| {
                Ok((
                    c.query_row(
                        "SELECT status FROM rename_requests WHERE id=?1",
                        params![sid],
                        |r| r.get(0),
                    )?,
                    c.query_row(
                        "SELECT retry_count FROM rename_requests WHERE id=?1",
                        params![sid],
                        |r| r.get(0),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(st, "PENDING");
        assert_eq!(rc, 1);
    }
}
