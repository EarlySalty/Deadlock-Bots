use std::sync::Arc;
use std::time::Duration;

use dl_db::{Db, DbError};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteamReadiness {
    Ready,
    MissingState,
    InvalidState,
    NotLoggedOn,
    GcNotReady,
}

#[derive(Debug)]
struct CompletedPublishTask {
    origin_hero_build_id: i64,
    target_language: i64,
    task_status: String,
    result: Option<String>,
    error: Option<String>,
    task_id: i64,
}

#[derive(Debug)]
struct BuildPublisher {
    db: Db,
    config: BuildPublisherConfig,
}

fn json_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn option_label(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "None".to_string())
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn steam_readiness(conn: &rusqlite::Connection) -> rusqlite::Result<SteamReadiness> {
    let payload = conn
        .query_row(
            "SELECT payload FROM standalone_bot_state WHERE bot='steam' LIMIT 1",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(payload) = payload.filter(|value| !value.trim().is_empty()) else {
        return Ok(SteamReadiness::MissingState);
    };
    let Ok(value) = serde_json::from_str::<Value>(&payload) else {
        return Ok(SteamReadiness::InvalidState);
    };
    let runtime = value.get("runtime").and_then(Value::as_object);
    let logged_on = runtime
        .and_then(|runtime| runtime.get("logged_on"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !logged_on {
        return Ok(SteamReadiness::NotLoggedOn);
    }
    let gc_ready = runtime
        .and_then(|runtime| runtime.get("deadlock_gc_ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !gc_ready {
        return Ok(SteamReadiness::GcNotReady);
    }
    Ok(SteamReadiness::Ready)
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
                match steam_readiness(conn)? {
                    SteamReadiness::Ready => {}
                    SteamReadiness::MissingState => {
                        stats.skipped = 1;
                        tracing::warn!(
                            "Build publisher skipped: Steam state missing in standalone_bot_state"
                        );
                        return Ok(stats);
                    }
                    SteamReadiness::InvalidState => {
                        stats.skipped = 1;
                        tracing::warn!(
                            "Build publisher skipped: Steam state payload is invalid JSON"
                        );
                        return Ok(stats);
                    }
                    SteamReadiness::NotLoggedOn => {
                        stats.skipped = 1;
                        tracing::warn!("Build publisher skipped: Steam not logged in");
                        return Ok(stats);
                    }
                    SteamReadiness::GcNotReady => {
                        stats.skipped = 1;
                        tracing::warn!("Build publisher skipped: Deadlock GC not ready");
                        return Ok(stats);
                    }
                }

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
                let mut statement = conn.prepare(
                    r#"
                    SELECT c.origin_hero_build_id, c.target_language,
                           t.status as task_status, t.result, t.error, t.id as task_id
                    FROM hero_build_clones c
                    INNER JOIN steam_tasks t ON (
                        t.type = 'BUILD_PUBLISH'
                        AND json_extract(t.payload, '$.origin_hero_build_id') = c.origin_hero_build_id
                    )
                    WHERE c.status = 'processing'
                      AND t.status IN ('DONE', 'FAILED')
                    ORDER BY t.finished_at DESC
                    "#,
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(CompletedPublishTask {
                            origin_hero_build_id: row.get("origin_hero_build_id")?,
                            target_language: row.get("target_language")?,
                            task_status: row.get("task_status")?,
                            result: row.get("result")?,
                            error: row.get("error")?,
                            task_id: row.get("task_id")?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                let mut stats = MonitorStats {
                    checked: rows.len(),
                    ..MonitorStats::default()
                };
                for row in rows {
                    match row.task_status.as_str() {
                        "DONE" => {
                            let result = row
                                .result
                                .as_deref()
                                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                                .unwrap_or(Value::Null);
                            let response = result.get("response");
                            let uploaded_id =
                                json_i64(response.and_then(|value| value.get("hero_build_id")));
                            let version = json_i64(response.and_then(|value| value.get("version")));
                            let changed = conn.execute(
                                r#"
                                UPDATE hero_build_clones
                                SET status = 'uploaded',
                                    uploaded_build_id = ?1,
                                    uploaded_version = ?2,
                                    status_info = ?3,
                                    updated_at = ?4
                                WHERE origin_hero_build_id = ?5
                                  AND target_language = ?6
                                  AND status = 'processing'
                                "#,
                                params![
                                    uploaded_id,
                                    version,
                                    format!(
                                        "Published as build #{} v{}",
                                        option_label(uploaded_id),
                                        option_label(version)
                                    ),
                                    now_ts,
                                    row.origin_hero_build_id,
                                    row.target_language,
                                ],
                            )?;
                            if changed > 0 {
                                stats.completed += 1;
                                tracing::info!(
                                    origin_hero_build_id = row.origin_hero_build_id,
                                    uploaded_build_id = uploaded_id,
                                    "Build published successfully"
                                );
                            }
                        }
                        "FAILED" => {
                            let error = row.error.unwrap_or_else(|| "Unknown error".to_string());
                            let changed = conn.execute(
                                r#"
                                UPDATE hero_build_clones
                                SET status = 'failed',
                                    status_info = ?1,
                                    updated_at = ?2
                                WHERE origin_hero_build_id = ?3
                                  AND target_language = ?4
                                  AND status = 'processing'
                                "#,
                                params![
                                    format!(
                                        "Task #{} failed: {}",
                                        row.task_id,
                                        truncate_chars(&error, 500)
                                    ),
                                    now_ts,
                                    row.origin_hero_build_id,
                                    row.target_language,
                                ],
                            )?;
                            if changed > 0 {
                                stats.failed += 1;
                                tracing::warn!(
                                    origin_hero_build_id = row.origin_hero_build_id,
                                    task_id = row.task_id,
                                    error = %truncate_chars(&error, 100),
                                    "Build publishing failed"
                                );
                            }
                        }
                        _ => {}
                    }
                }

                let stale_threshold = now_ts - (30 * 60);
                stats.reset_stale = conn.execute(
                    r#"
                    UPDATE hero_build_clones
                    SET status = 'pending',
                        status_info = ?1,
                        attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END
                    WHERE status = 'processing'
                      AND last_attempt_at < ?2
                      AND last_attempt_at IS NOT NULL
                      AND NOT EXISTS (
                          SELECT 1
                          FROM steam_tasks t
                          WHERE t.type = 'BUILD_PUBLISH'
                            AND json_extract(t.payload, '$.origin_hero_build_id') = hero_build_clones.origin_hero_build_id
                            AND t.status IN ('DONE', 'FAILED')
                      )
                    "#,
                    params![STATUS_INFO_RESET_STALE, stale_threshold],
                )?;

                Ok(stats)
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
                        if stats.completed > 0 || stats.failed > 0 {
                            tracing::info!(
                                completed = stats.completed,
                                failed = stats.failed,
                                checked = stats.checked,
                                "Build publisher monitor completed"
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
            uploaded_build_id INTEGER,
            uploaded_version INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER,
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
            status TEXT NOT NULL DEFAULT 'PENDING',
            result TEXT,
            error TEXT,
            finished_at INTEGER
        );
        CREATE TABLE standalone_bot_state(
            bot TEXT PRIMARY KEY,
            heartbeat INTEGER NOT NULL,
            payload TEXT,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
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

    async fn set_steam_state(db: &Db, logged_on: bool, gc_ready: bool) -> Result<(), DbError> {
        db.write(move |conn| {
            let payload = json!({
                "runtime": {
                    "logged_on": logged_on,
                    "deadlock_gc_ready": gc_ready,
                }
            })
            .to_string();
            conn.execute(
                r#"
                INSERT INTO standalone_bot_state(bot, heartbeat, payload)
                VALUES('steam', 1, ?1)
                ON CONFLICT(bot) DO UPDATE SET payload = excluded.payload
                "#,
                params![payload],
            )?;
            Ok(())
        })
        .await
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
        set_steam_state(&db, true, true).await?;
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
        set_steam_state(&db, true, true).await?;
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
        set_steam_state(&db, true, true).await?;
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
    async fn process_queue_skips_when_steam_login_or_gc_ready_is_missing() -> TestResult {
        for (logged_on, gc_ready) in [(false, true), (true, false)] {
            let (_dir, db) = test_db().await?;
            set_steam_state(&db, logged_on, gc_ready).await?;
            insert_clone(&db, 350, 1, 35, Some(0), 100, 10, 0).await?;

            let stats = publisher(db.clone())
                .process_queue_at(4_000, "test")
                .await?;
            assert_eq!(stats.skipped, 1);
            assert_eq!(stats.checked, 0);
            assert_eq!(stats.queued, 0);
            assert_eq!(stats.cancelled_excess, 0);

            let (task_count, status) = db
                .read(|conn| {
                    let task_count: i64 =
                        conn.query_row("SELECT COUNT(*) FROM steam_tasks", [], |row| row.get(0))?;
                    let status: String = conn.query_row(
                        "SELECT status FROM hero_build_clones WHERE origin_hero_build_id = 350",
                        [],
                        |row| row.get(0),
                    )?;
                    Ok((task_count, status))
                })
                .await?;
            assert_eq!(task_count, 0);
            assert_eq!(status, "pending");
        }

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

    #[tokio::test]
    async fn monitor_applies_completed_and_failed_build_publish_tasks() -> TestResult {
        let (_dir, db) = test_db().await?;
        db.write(|conn| {
            conn.execute(
                r#"
                INSERT INTO hero_build_clones(
                    origin_hero_build_id, hero_id, target_language, status,
                    created_at, updated_at, last_attempt_at, attempts
                ) VALUES
                    (500, 1, 2, 'processing', 1, 1, 9500, 1),
                    (501, 1, 2, 'processing', 1, 1, 9500, 1)
                "#,
                [],
            )?;
            conn.execute(
                r#"
                INSERT INTO steam_tasks(id, type, payload, status, result, finished_at)
                VALUES(900, 'BUILD_PUBLISH', ?1, 'DONE', ?2, 9_900)
                "#,
                params![
                    json!({"origin_hero_build_id": 500}).to_string(),
                    json!({"response": {"hero_build_id": 7000, "version": 12}}).to_string(),
                ],
            )?;
            conn.execute(
                r#"
                INSERT INTO steam_tasks(id, type, payload, status, error, finished_at)
                VALUES(901, 'BUILD_PUBLISH', ?1, 'FAILED', 'upload failed hard', 9_901)
                "#,
                params![json!({"origin_hero_build_id": 501}).to_string()],
            )?;
            Ok(())
        })
        .await?;

        let stats = publisher(db.clone()).monitor_tasks_at(10_000).await?;
        assert_eq!(stats.checked, 2);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.reset_stale, 0);

        let rows = db
            .read(|conn| {
                let mut statement = conn.prepare(
                    r#"
                    SELECT origin_hero_build_id, status, uploaded_build_id,
                           uploaded_version, status_info, updated_at
                    FROM hero_build_clones
                    ORDER BY origin_hero_build_id
                    "#,
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<i64>>(5)?,
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
                    500,
                    "uploaded".to_string(),
                    Some(7000),
                    Some(12),
                    Some("Published as build #7000 v12".to_string()),
                    Some(10_000),
                ),
                (
                    501,
                    "failed".to_string(),
                    None,
                    None,
                    Some("Task #901 failed: upload failed hard".to_string()),
                    Some(10_000),
                ),
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn monitor_consumes_finished_stale_task_before_resetting_processing_clone() -> TestResult
    {
        let (_dir, db) = test_db().await?;
        db.write(|conn| {
            conn.execute(
                r#"
                INSERT INTO hero_build_clones(
                    origin_hero_build_id, hero_id, target_language, status,
                    created_at, updated_at, last_attempt_at, attempts
                ) VALUES(600, 1, 2, 'processing', 1, 1, 1_000, 2)
                "#,
                [],
            )?;
            conn.execute(
                r#"
                INSERT INTO steam_tasks(id, type, payload, status, result, finished_at)
                VALUES(910, 'BUILD_PUBLISH', ?1, 'DONE', ?2, 2_500)
                "#,
                params![
                    json!({"origin_hero_build_id": 600}).to_string(),
                    json!({"response": {"hero_build_id": 7600, "version": 3}}).to_string(),
                ],
            )?;
            Ok(())
        })
        .await?;

        let stats = publisher(db.clone()).monitor_tasks_at(3_000).await?;
        assert_eq!(stats.checked, 1);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.reset_stale, 0);

        let row = db
            .read(|conn| {
                conn.query_row(
                    r#"
                    SELECT status, attempts, uploaded_build_id, uploaded_version, status_info
                    FROM hero_build_clones
                    WHERE origin_hero_build_id = 600
                    "#,
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
            })
            .await?;
        assert_eq!(
            row,
            (
                "uploaded".to_string(),
                2,
                Some(7600),
                Some(3),
                Some("Published as build #7600 v3".to_string()),
            )
        );

        Ok(())
    }

    #[tokio::test]
    async fn monitor_applies_only_latest_completed_task_for_a_processing_clone() -> TestResult {
        let (_dir, db) = test_db().await?;
        db.write(|conn| {
            conn.execute(
                r#"
                INSERT INTO hero_build_clones(
                    origin_hero_build_id, hero_id, target_language, status,
                    created_at, updated_at, last_attempt_at, attempts
                ) VALUES(610, 1, 2, 'processing', 1, 1, 2_500, 1)
                "#,
                [],
            )?;
            conn.execute(
                r#"
                INSERT INTO steam_tasks(id, type, payload, status, result, finished_at)
                VALUES(920, 'BUILD_PUBLISH', ?1, 'DONE', ?2, 2_900)
                "#,
                params![
                    json!({"origin_hero_build_id": 610}).to_string(),
                    json!({"response": {"hero_build_id": 7610, "version": 1}}).to_string(),
                ],
            )?;
            conn.execute(
                r#"
                INSERT INTO steam_tasks(id, type, payload, status, result, finished_at)
                VALUES(921, 'BUILD_PUBLISH', ?1, 'DONE', ?2, 2_950)
                "#,
                params![
                    json!({"origin_hero_build_id": 610}).to_string(),
                    json!({"response": {"hero_build_id": 7611, "version": 2}}).to_string(),
                ],
            )?;
            Ok(())
        })
        .await?;

        let stats = publisher(db.clone()).monitor_tasks_at(3_000).await?;
        assert_eq!(stats.checked, 2);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.reset_stale, 0);

        let row = db
            .read(|conn| {
                conn.query_row(
                    r#"
                    SELECT status, uploaded_build_id, uploaded_version, status_info
                    FROM hero_build_clones
                    WHERE origin_hero_build_id = 610
                    "#,
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
            })
            .await?;
        assert_eq!(
            row,
            (
                "uploaded".to_string(),
                Some(7611),
                Some(2),
                Some("Published as build #7611 v2".to_string()),
            )
        );

        Ok(())
    }
}
