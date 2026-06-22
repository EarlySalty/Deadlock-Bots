use std::sync::Arc;
use std::time::Duration;

use dl_db::{Db, DbError};
use rusqlite::params;
use serde_json::json;

const STATUS_INFO_CANCELLED_EXCESS: &str =
    "Cancelled: Excluded by 3-build-per-hero rule based on priority/recency.";
const STATUS_INFO_RESET_STALE: &str = "Reset: stuck in processing for >30min";

#[derive(Debug, Clone, Copy)]
struct BuildPublisherConfig {
    publish_interval_seconds: u64,
    monitor_interval_seconds: u64,
    max_attempts: i64,
    batch_size: i64,
}

impl BuildPublisherConfig {
    fn production() -> Self {
        Self {
            publish_interval_seconds: 10 * 60,
            monitor_interval_seconds: 2 * 60,
            max_attempts: 3,
            batch_size: 5,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QueueStats {
    checked: usize,
    queued: usize,
    skipped: usize,
    errors: usize,
    cancelled_excess: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MonitorStats {
    checked: usize,
    completed: usize,
    failed: usize,
    reset_stale: usize,
}

#[derive(Debug, Clone)]
struct PendingClone {
    origin_hero_build_id: i64,
    hero_id: i64,
    target_language: i64,
    target_name: Option<String>,
    target_description: Option<String>,
    attempts: i64,
}

#[derive(Debug)]
struct BuildPublisher {
    db: Db,
    config: BuildPublisherConfig,
}

impl BuildPublisher {
    fn new(db: Db, config: BuildPublisherConfig) -> Self {
        Self { db, config }
    }

    async fn process_queue(&self, triggered_by: &'static str) -> Result<QueueStats, DbError> {
        self.process_queue_at(chrono::Utc::now().timestamp(), triggered_by)
            .await
    }

    async fn process_queue_at(
        &self,
        now_ts: i64,
        triggered_by: &'static str,
    ) -> Result<QueueStats, DbError> {
        let config = self.config;
        let stats = self
            .db
            .write(move |conn| {
                let mut stats = QueueStats::default();
                let mut statement = conn.prepare(
                    r#"
                    WITH RankedClones AS (
                        SELECT
                            hbc.origin_hero_build_id,
                            hbc.hero_id,
                            hbc.target_language,
                            hbc.target_name,
                            hbc.target_description,
                            hbc.attempts,
                            hbs.publish_ts,
                            wba.priority,
                            hbc.created_at,
                            ROW_NUMBER() OVER(PARTITION BY hbc.hero_id ORDER BY COALESCE(wba.priority, 0) ASC, hbs.publish_ts DESC, hbc.created_at ASC) as rn
                        FROM hero_build_clones hbc
                        INNER JOIN hero_build_sources hbs ON hbc.origin_hero_build_id = hbs.hero_build_id
                        LEFT JOIN watched_build_authors wba ON hbs.author_account_id = wba.author_account_id
                        WHERE hbc.status = 'pending'
                          AND hbc.attempts < ?1
                    )
                    SELECT origin_hero_build_id, hero_id, target_language,
                           target_name, target_description, attempts
                    FROM RankedClones
                    WHERE rn <= 3
                    ORDER BY created_at ASC
                    LIMIT ?2
                    "#,
                )?;
                let rows = statement
                    .query_map(params![config.max_attempts, config.batch_size], |row| {
                        Ok(PendingClone {
                            origin_hero_build_id: row.get("origin_hero_build_id")?,
                            hero_id: row.get("hero_id")?,
                            target_language: row.get("target_language")?,
                            target_name: row.get("target_name")?,
                            target_description: row.get("target_description")?,
                            attempts: row.get("attempts")?,
                        })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
                stats.checked = rows.len();

                stats.cancelled_excess = conn.execute(
                    r#"
                    WITH RankedPendingClones AS (
                        SELECT
                            hbc.ROWID,
                            ROW_NUMBER() OVER(PARTITION BY hbc.hero_id ORDER BY COALESCE(wba.priority, 0) ASC, hbs.publish_ts DESC, hbc.created_at ASC) as rn
                        FROM hero_build_clones hbc
                        INNER JOIN hero_build_sources hbs ON hbc.origin_hero_build_id = hbs.hero_build_id
                        LEFT JOIN watched_build_authors wba ON hbs.author_account_id = wba.author_account_id
                        WHERE hbc.status = 'pending'
                          AND hbc.attempts < ?1
                    )
                    UPDATE hero_build_clones
                    SET status = 'cancelled',
                        status_info = ?2
                    WHERE ROWID IN (SELECT ROWID FROM RankedPendingClones WHERE rn > 3)
                    "#,
                    params![config.max_attempts, STATUS_INFO_CANCELLED_EXCESS],
                )?;

                for row in rows {
                    let payload = json!({
                        "origin_hero_build_id": row.origin_hero_build_id,
                        "target_name": row.target_name,
                        "target_description": row.target_description,
                        "target_language": row.target_language,
                        "minimal": row.attempts == 0,
                    })
                    .to_string();
                    conn.execute(
                        "INSERT INTO steam_tasks(type, payload, status) VALUES(?1, ?2, 'PENDING')",
                        params!["BUILD_PUBLISH", payload],
                    )?;
                    let task_id = conn.last_insert_rowid();
                    conn.execute(
                        r#"
                        UPDATE hero_build_clones
                        SET status = 'processing',
                            last_attempt_at = ?1,
                            attempts = attempts + 1,
                            status_info = ?2
                        WHERE origin_hero_build_id = ?3
                          AND target_language = ?4
                        "#,
                        params![
                            now_ts,
                            format!("Task #{task_id} created"),
                            row.origin_hero_build_id,
                            row.target_language,
                        ],
                    )?;
                    stats.queued += 1;
                    tracing::info!(
                        task_id,
                        origin_hero_build_id = row.origin_hero_build_id,
                        hero_id = row.hero_id,
                        attempts = row.attempts + 1,
                        "BUILD_PUBLISH task created"
                    );
                }

                let stats_json = json!({
                    "checked": stats.checked,
                    "queued": stats.queued,
                    "skipped": stats.skipped,
                    "errors": stats.errors,
                    "cancelled_excess": stats.cancelled_excess,
                })
                .to_string();
                conn.execute(
                    r#"
                    INSERT INTO kv_store(ns, k, v) VALUES('build_publisher', 'last_run_ts', ?1)
                    ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v
                    "#,
                    params![now_ts.to_string()],
                )?;
                conn.execute(
                    r#"
                    INSERT INTO kv_store(ns, k, v) VALUES('build_publisher', 'last_run_stats', ?1)
                    ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v
                    "#,
                    params![stats_json],
                )?;
                conn.execute(
                    r#"
                    INSERT INTO kv_store(ns, k, v) VALUES('build_publisher', 'last_run_trigger', ?1)
                    ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v
                    "#,
                    params![triggered_by],
                )?;

                Ok(stats)
            })
            .await;

        if let Err(error) = &stats {
            let message = error.to_string();
            let set_result = self
                .db
                .write(move |conn| {
                    conn.execute(
                        r#"
                        INSERT INTO kv_store(ns, k, v) VALUES('build_publisher', 'last_error', ?1)
                        ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v
                        "#,
                        params![message],
                    )
                    .map(|_| ())
                })
                .await;
            if let Err(kv_error) = set_result {
                tracing::warn!(%kv_error, "Build publisher last_error konnte nicht geschrieben werden");
            }
        }

        stats
    }

    async fn monitor_tasks(&self) -> Result<MonitorStats, DbError> {
        self.monitor_tasks_at(chrono::Utc::now().timestamp()).await
    }

    async fn monitor_tasks_at(&self, now_ts: i64) -> Result<MonitorStats, DbError> {
        self.db
            .write(move |conn| {
                let stale_threshold = now_ts - (30 * 60);
                let reset_stale = conn.execute(
                    r#"
                    UPDATE hero_build_clones
                    SET status = 'pending',
                        status_info = ?1,
                        attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END
                    WHERE status = 'processing'
                      AND last_attempt_at < ?2
                      AND last_attempt_at IS NOT NULL
                    "#,
                    params![STATUS_INFO_RESET_STALE, stale_threshold],
                )?;

                Ok(MonitorStats {
                    reset_stale,
                    ..MonitorStats::default()
                })
            })
            .await
    }
}

pub fn spawn(db: Db) -> Vec<tokio::task::JoinHandle<()>> {
    let publisher = Arc::new(BuildPublisher::new(db, BuildPublisherConfig::production()));
    let publisher_loop = {
        let publisher = publisher.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            loop {
                match publisher.process_queue("auto").await {
                    Ok(stats) => {
                        if stats.queued > 0 || stats.errors > 0 {
                            tracing::info!(
                                queued = stats.queued,
                                errors = stats.errors,
                                checked = stats.checked,
                                "Build publisher run completed"
                            );
                        }
                        if stats.cancelled_excess > 0 {
                            tracing::info!(
                                cancelled_excess = stats.cancelled_excess,
                                "Build publisher cancelled excess pending builds"
                            );
                        }
                    }
                    Err(error) => tracing::error!(%error, "Build publisher run failed"),
                }
                tokio::time::sleep(Duration::from_secs(
                    publisher.config.publish_interval_seconds,
                ))
                .await;
            }
        })
    };

    let monitor_loop = {
        let publisher = publisher.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                match publisher.monitor_tasks().await {
                    Ok(stats) => {
                        if stats.reset_stale > 0 {
                            tracing::warn!(
                                reset_stale = stats.reset_stale,
                                "Reset stale builds stuck in processing state"
                            );
                        }
                    }
                    Err(error) => tracing::error!(%error, "Build monitor run failed"),
                }
                tokio::time::sleep(Duration::from_secs(
                    publisher.config.monitor_interval_seconds,
                ))
                .await;
            }
        })
    };

    vec![publisher_loop, monitor_loop]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::Value;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const TEST_DDL: &str = r#"
        CREATE TABLE kv_store(
            ns TEXT NOT NULL,
            k TEXT NOT NULL,
            v TEXT NOT NULL,
            PRIMARY KEY(ns, k)
        );
        CREATE TABLE hero_build_clones(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            origin_hero_build_id INTEGER NOT NULL,
            hero_id INTEGER NOT NULL,
            target_language INTEGER NOT NULL,
            target_name TEXT,
            target_description TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            status_info TEXT,
            created_at INTEGER NOT NULL,
            last_attempt_at INTEGER,
            attempts INTEGER NOT NULL DEFAULT 0,
            UNIQUE(origin_hero_build_id, target_language)
        );
        CREATE TABLE hero_build_sources(
            hero_build_id INTEGER PRIMARY KEY,
            author_account_id INTEGER NOT NULL,
            hero_id INTEGER NOT NULL,
            language INTEGER NOT NULL,
            version INTEGER NOT NULL,
            name TEXT NOT NULL,
            publish_ts INTEGER
        );
        CREATE TABLE watched_build_authors(
            author_account_id INTEGER PRIMARY KEY NOT NULL,
            priority INTEGER
        );
        CREATE TABLE steam_tasks(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            type TEXT NOT NULL,
            payload TEXT,
            status TEXT NOT NULL DEFAULT 'PENDING'
        );
    "#;

    async fn test_db() -> Result<(tempfile::TempDir, Db), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = Db::open_creating(dir.path().join("test.sqlite3"))?;
        db.write(|conn| conn.execute_batch(TEST_DDL)).await?;
        Ok((dir, db))
    }

    fn publisher(db: Db) -> BuildPublisher {
        BuildPublisher::new(
            db,
            BuildPublisherConfig {
                publish_interval_seconds: 10 * 60,
                monitor_interval_seconds: 2 * 60,
                max_attempts: 3,
                batch_size: 5,
            },
        )
    }

    async fn insert_clone(
        db: &Db,
        origin_hero_build_id: i64,
        hero_id: i64,
        author_account_id: i64,
        priority: Option<i64>,
        publish_ts: i64,
        created_at: i64,
        attempts: i64,
    ) -> Result<(), DbError> {
        db.write(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO watched_build_authors(author_account_id, priority) VALUES(?1, ?2)",
                params![author_account_id, priority],
            )?;
            conn.execute(
                r#"
                INSERT INTO hero_build_sources(
                    hero_build_id, author_account_id, hero_id, language, version, name, publish_ts
                ) VALUES(?1, ?2, ?3, 1, 1, ?4, ?5)
                "#,
                params![
                    origin_hero_build_id,
                    author_account_id,
                    hero_id,
                    format!("source-{origin_hero_build_id}"),
                    publish_ts,
                ],
            )?;
            conn.execute(
                r#"
                INSERT INTO hero_build_clones(
                    origin_hero_build_id, hero_id, target_language, target_name,
                    target_description, created_at, attempts
                ) VALUES(?1, ?2, 2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    origin_hero_build_id,
                    hero_id,
                    format!("target-{origin_hero_build_id}"),
                    format!("description-{origin_hero_build_id}"),
                    created_at,
                    attempts,
                ],
            )?;
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn process_queue_uses_ranked_top_three_then_processes_oldest_valid_tasks() -> TestResult {
        let (_dir, db) = test_db().await?;
        insert_clone(&db, 100, 1, 10, Some(0), 100, 50, 0).await?;
        insert_clone(&db, 101, 1, 11, Some(0), 200, 40, 0).await?;
        insert_clone(&db, 102, 1, 12, Some(1), 300, 10, 0).await?;
        insert_clone(&db, 103, 1, 13, Some(1), 100, 20, 0).await?;
        insert_clone(&db, 104, 1, 14, None, 150, 30, 0).await?;

        let stats = publisher(db.clone())
            .process_queue_at(1_000, "test")
            .await?;
        assert_eq!(stats.checked, 3);
        assert_eq!(stats.queued, 3);
        assert_eq!(stats.cancelled_excess, 2);

        let payloads = db
            .read(|conn| {
                let mut statement = conn.prepare("SELECT payload FROM steam_tasks ORDER BY id")?;
                let payloads = statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(payloads)
            })
            .await?;
        let origins = payloads
            .iter()
            .map(|payload| {
                let value: Value = serde_json::from_str(payload)?;
                Ok(value
                    .get("origin_hero_build_id")
                    .and_then(Value::as_i64)
                    .unwrap_or_default())
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        assert_eq!(origins, vec![104, 101, 100]);

        Ok(())
    }

    #[tokio::test]
    async fn process_queue_obeys_attempt_limits_batch_size_and_minimal_mode() -> TestResult {
        let (_dir, db) = test_db().await?;
        insert_clone(&db, 200, 1, 20, Some(0), 200, 10, 0).await?;
        insert_clone(&db, 201, 1, 21, Some(0), 190, 20, 1).await?;
        insert_clone(&db, 202, 1, 22, Some(0), 180, 30, 2).await?;
        insert_clone(&db, 203, 2, 23, Some(0), 200, 40, 0).await?;
        insert_clone(&db, 204, 2, 24, Some(0), 190, 50, 0).await?;
        insert_clone(&db, 205, 2, 25, Some(0), 180, 60, 0).await?;
        insert_clone(&db, 206, 3, 26, Some(0), 200, 70, 3).await?;

        let stats = publisher(db.clone())
            .process_queue_at(2_000, "test")
            .await?;
        assert_eq!(stats.checked, 5);
        assert_eq!(stats.queued, 5);

        let rows = db
            .read(|conn| {
                let mut statement = conn.prepare(
                    r#"
                    SELECT c.origin_hero_build_id, c.status, c.attempts, t.payload
                    FROM hero_build_clones c
                    LEFT JOIN steam_tasks t ON json_extract(t.payload, '$.origin_hero_build_id') = c.origin_hero_build_id
                    ORDER BY c.origin_hero_build_id
                    "#,
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;

        let retry_payload = rows
            .iter()
            .find_map(|(origin, _, _, payload)| (*origin == 201).then_some(payload))
            .and_then(|payload| payload.as_ref())
            .ok_or("retry payload missing")?;
        let retry_value: Value = serde_json::from_str(retry_payload)?;
        assert_eq!(retry_value.get("minimal"), Some(&Value::Bool(false)));

        let first_payload = rows
            .iter()
            .find_map(|(origin, _, _, payload)| (*origin == 200).then_some(payload))
            .and_then(|payload| payload.as_ref())
            .ok_or("first payload missing")?;
        let first_value: Value = serde_json::from_str(first_payload)?;
        assert_eq!(first_value.get("minimal"), Some(&Value::Bool(true)));

        let max_attempts_row = rows
            .iter()
            .find(|(origin, _, _, _)| *origin == 206)
            .ok_or("max-attempts row missing")?;
        assert_eq!(max_attempts_row.1, "pending");
        assert_eq!(max_attempts_row.2, 3);
        assert!(max_attempts_row.3.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn process_queue_cancels_pending_overflow_per_hero() -> TestResult {
        let (_dir, db) = test_db().await?;
        for idx in 0..5 {
            insert_clone(&db, 300 + idx, 7, 30 + idx, Some(0), 500 - idx, 10 + idx, 0).await?;
        }

        let stats = publisher(db.clone())
            .process_queue_at(3_000, "test")
            .await?;
        assert_eq!(stats.cancelled_excess, 2);

        let cancelled = db
            .read(|conn| {
                let mut statement = conn.prepare(
                    r#"
                    SELECT origin_hero_build_id, status_info
                    FROM hero_build_clones
                    WHERE status = 'cancelled'
                    ORDER BY origin_hero_build_id
                    "#,
                )?;
                let cancelled = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(cancelled)
            })
            .await?;

        assert_eq!(
            cancelled,
            vec![
                (303, STATUS_INFO_CANCELLED_EXCESS.to_string()),
                (304, STATUS_INFO_CANCELLED_EXCESS.to_string()),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn monitor_resets_only_processing_clones_older_than_thirty_minutes() -> TestResult {
        let (_dir, db) = test_db().await?;
        db.write(|conn| {
            conn.execute(
                r#"
                INSERT INTO hero_build_clones(
                    origin_hero_build_id, hero_id, target_language, status,
                    created_at, last_attempt_at, attempts
                ) VALUES
                    (400, 1, 2, 'processing', 1, 1199, 2),
                    (401, 1, 2, 'processing', 1, 1200, 2),
                    (402, 1, 2, 'processing', 1, NULL, 2),
                    (403, 1, 2, 'pending', 1, 1100, 2),
                    (404, 1, 2, 'processing', 1, 1100, 0)
                "#,
                [],
            )?;
            Ok(())
        })
        .await?;

        let stats = publisher(db.clone()).monitor_tasks_at(3_000).await?;
        assert_eq!(stats.reset_stale, 2);

        let rows = db
            .read(|conn| {
                let mut statement = conn.prepare(
                    r#"
                    SELECT origin_hero_build_id, status, status_info, attempts
                    FROM hero_build_clones
                    ORDER BY origin_hero_build_id
                    "#,
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;

        assert_eq!(
            rows,
            vec![
                (
                    400,
                    "pending".to_string(),
                    Some(STATUS_INFO_RESET_STALE.to_string()),
                    1
                ),
                (401, "processing".to_string(), None, 2),
                (402, "processing".to_string(), None, 2),
                (403, "pending".to_string(), None, 2),
                (
                    404,
                    "pending".to_string(),
                    Some(STATUS_INFO_RESET_STALE.to_string()),
                    0
                ),
            ]
        );

        Ok(())
    }
}
