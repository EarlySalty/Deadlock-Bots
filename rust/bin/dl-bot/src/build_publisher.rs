use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;

const STATUS_INFO_CANCELLED_EXCESS: &str =
    "Cancelled: Excluded by 3-build-per-hero rule based on priority/recency.";
const STATUS_INFO_RESET_STALE: &str = "Reset: stuck in processing for >30min";

#[derive(Debug, Clone, Copy)]
struct BuildPublisherConfig {
    publish_interval_seconds: u64,
    monitor_interval_seconds: u64,
    max_attempts: i32,
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
    target_language: i32,
    target_name: Option<String>,
    target_description: Option<String>,
    attempts: i32,
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
    target_language: i32,
    task_status: String,
    result: Option<String>,
    error: Option<String>,
    task_id: i64,
}

#[derive(Debug)]
struct BuildPublisher {
    pool: PgPool,
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

fn timestamp_seconds(value: i64) -> anyhow::Result<DateTime<Utc>> {
    Utc.timestamp_opt(value, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("ungueltiger Unix-Timestamp: {value}"))
}

fn rows_affected_to_usize(value: u64) -> anyhow::Result<usize> {
    usize::try_from(value).map_err(|_| anyhow::anyhow!("rows_affected passt nicht in usize"))
}

async fn steam_readiness(pool: &PgPool) -> anyhow::Result<SteamReadiness> {
    let row = sqlx::query!(
        r#"
        SELECT payload::text AS "payload?"
          FROM bot.standalone_bot_state
         WHERE bot = 'steam'
         LIMIT 1
        "#
    )
    .fetch_optional(pool)
    .await?;
    let payload = row.and_then(|row| row.payload);
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
    fn new(pool: PgPool, config: BuildPublisherConfig) -> Self {
        Self { pool, config }
    }

    async fn process_queue(&self, triggered_by: &'static str) -> anyhow::Result<QueueStats> {
        self.process_queue_at(chrono::Utc::now().timestamp(), triggered_by)
            .await
    }

    async fn process_queue_at(
        &self,
        now_ts: i64,
        triggered_by: &'static str,
    ) -> anyhow::Result<QueueStats> {
        let result = self.process_queue_inner(now_ts, triggered_by).await;
        if let Err(error) = &result {
            if let Err(kv_error) = self.set_kv("last_error", &error.to_string()).await {
                tracing::warn!(%kv_error, "Build publisher last_error konnte nicht geschrieben werden");
            }
        }
        result
    }

    async fn process_queue_inner(
        &self,
        now_ts: i64,
        triggered_by: &'static str,
    ) -> anyhow::Result<QueueStats> {
        let mut stats = QueueStats::default();
        match steam_readiness(&self.pool).await? {
            SteamReadiness::Ready => {}
            SteamReadiness::MissingState => {
                stats.skipped = 1;
                tracing::warn!(
                    "Build publisher skipped: Steam state missing in bot.standalone_bot_state"
                );
                return Ok(stats);
            }
            SteamReadiness::InvalidState => {
                stats.skipped = 1;
                tracing::warn!("Build publisher skipped: Steam state payload is invalid JSON");
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

        let config = self.config;
        let now = timestamp_seconds(now_ts)?;
        let mut tx = self.pool.begin().await?;

        let rows = sqlx::query_as!(
            PendingClone,
            r#"
            WITH ranked_clones AS (
                SELECT
                    hbc.origin_hero_build_id,
                    hbc.hero_id,
                    hbc.target_language,
                    hbc.target_name,
                    hbc.target_description,
                    hbc.attempts,
                    hbc.created_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY hbc.hero_id
                        ORDER BY COALESCE(wba.priority, 0) ASC,
                                 hbs.published_at DESC NULLS LAST,
                                 hbc.created_at ASC
                    ) AS rn
                FROM tierlist.hero_build_clones hbc
                INNER JOIN tierlist.hero_build_sources hbs
                        ON hbc.origin_hero_build_id = hbs.hero_build_id
                LEFT JOIN tierlist.watched_build_authors wba
                       ON hbs.author_account_id = wba.author_account_id
                WHERE hbc.status = 'pending'
                  AND hbc.attempts < $1
            )
            SELECT origin_hero_build_id, hero_id, target_language,
                   target_name, target_description, attempts
              FROM ranked_clones
             WHERE rn <= 3
             ORDER BY created_at ASC
             LIMIT $2
            "#,
            config.max_attempts,
            config.batch_size,
        )
        .fetch_all(&mut *tx)
        .await?;
        stats.checked = rows.len();

        let cancelled = sqlx::query!(
            r#"
            WITH ranked_pending_clones AS (
                SELECT
                    hbc.id,
                    ROW_NUMBER() OVER (
                        PARTITION BY hbc.hero_id
                        ORDER BY COALESCE(wba.priority, 0) ASC,
                                 hbs.published_at DESC NULLS LAST,
                                 hbc.created_at ASC
                    ) AS rn
                FROM tierlist.hero_build_clones hbc
                INNER JOIN tierlist.hero_build_sources hbs
                        ON hbc.origin_hero_build_id = hbs.hero_build_id
                LEFT JOIN tierlist.watched_build_authors wba
                       ON hbs.author_account_id = wba.author_account_id
                WHERE hbc.status = 'pending'
                  AND hbc.attempts < $1
            )
            UPDATE tierlist.hero_build_clones hbc
               SET status = 'cancelled',
                   status_info = $2,
                   updated_at = $3
              FROM ranked_pending_clones ranked
             WHERE hbc.id = ranked.id
               AND ranked.rn > 3
            "#,
            config.max_attempts,
            STATUS_INFO_CANCELLED_EXCESS,
            now,
        )
        .execute(&mut *tx)
        .await?;
        stats.cancelled_excess = rows_affected_to_usize(cancelled.rows_affected())?;

        for row in rows {
            let payload = json!({
                "origin_hero_build_id": row.origin_hero_build_id,
                "target_name": row.target_name,
                "target_description": row.target_description,
                "target_language": row.target_language,
                "minimal": row.attempts == 0,
            })
            .to_string();
            let task = sqlx::query!(
                r#"
                INSERT INTO steam.steam_tasks(type, payload, status)
                VALUES('BUILD_PUBLISH', $1::text::jsonb, 'PENDING')
                RETURNING id
                "#,
                payload,
            )
            .fetch_one(&mut *tx)
            .await?;
            let task_id = task.id;
            let updated = sqlx::query!(
                r#"
                UPDATE tierlist.hero_build_clones
                   SET status = 'processing',
                       last_attempt_at = $1,
                       attempts = attempts + 1,
                       status_info = $2,
                       updated_at = $1
                 WHERE origin_hero_build_id = $3
                   AND target_language = $4
                   AND status = 'pending'
                "#,
                now,
                format!("Task #{task_id} created"),
                row.origin_hero_build_id,
                row.target_language,
            )
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() > 0 {
                stats.queued += 1;
                tracing::info!(
                    task_id,
                    origin_hero_build_id = row.origin_hero_build_id,
                    hero_id = row.hero_id,
                    attempts = row.attempts + 1,
                    "BUILD_PUBLISH task created"
                );
            }
        }

        let stats_json = json!({
            "checked": stats.checked,
            "queued": stats.queued,
            "skipped": stats.skipped,
            "errors": stats.errors,
            "cancelled_excess": stats.cancelled_excess,
        })
        .to_string();
        upsert_kv_tx(&mut tx, "last_run_ts", &now_ts.to_string()).await?;
        upsert_kv_tx(&mut tx, "last_run_stats", &stats_json).await?;
        upsert_kv_tx(&mut tx, "last_run_trigger", triggered_by).await?;

        tx.commit().await?;
        Ok(stats)
    }

    async fn set_kv(&self, key: &str, value: &str) -> anyhow::Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO bot.kv_store(ns, k, v)
            VALUES('build_publisher', $1, $2)
            ON CONFLICT(ns, k) DO UPDATE SET v = EXCLUDED.v
            "#,
            key,
            value,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn monitor_tasks(&self) -> anyhow::Result<MonitorStats> {
        self.monitor_tasks_at(chrono::Utc::now().timestamp()).await
    }

    async fn monitor_tasks_at(&self, now_ts: i64) -> anyhow::Result<MonitorStats> {
        let now = timestamp_seconds(now_ts)?;
        let stale_threshold = timestamp_seconds(now_ts - (30 * 60))?;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query_as!(
            CompletedPublishTask,
            r#"
            SELECT c.origin_hero_build_id,
                   c.target_language,
                   t.status AS task_status,
                   t.result::text AS "result?",
                   t.error,
                   t.id AS task_id
              FROM tierlist.hero_build_clones c
              INNER JOIN steam.steam_tasks t
                      ON t.type = 'BUILD_PUBLISH'
                     AND t.payload ? 'origin_hero_build_id'
                     AND t.payload->>'origin_hero_build_id' ~ '^-?[0-9]+$'
                     AND (t.payload->>'origin_hero_build_id')::BIGINT = c.origin_hero_build_id
             WHERE c.status = 'processing'
               AND t.status IN ('DONE', 'FAILED')
             ORDER BY t.finished_at DESC NULLS LAST, t.id DESC
            "#
        )
        .fetch_all(&mut *tx)
        .await?;

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
                    let changed = sqlx::query!(
                        r#"
                        UPDATE tierlist.hero_build_clones
                           SET status = 'uploaded',
                               uploaded_build_id = $1,
                               uploaded_version = $2,
                               status_info = $3,
                               updated_at = $4
                         WHERE origin_hero_build_id = $5
                           AND target_language = $6
                           AND status = 'processing'
                        "#,
                        uploaded_id,
                        version,
                        format!(
                            "Published as build #{} v{}",
                            option_label(uploaded_id),
                            option_label(version)
                        ),
                        now,
                        row.origin_hero_build_id,
                        row.target_language,
                    )
                    .execute(&mut *tx)
                    .await?;
                    if changed.rows_affected() > 0 {
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
                    let changed = sqlx::query!(
                        r#"
                        UPDATE tierlist.hero_build_clones
                           SET status = 'failed',
                               status_info = $1,
                               updated_at = $2
                         WHERE origin_hero_build_id = $3
                           AND target_language = $4
                           AND status = 'processing'
                        "#,
                        format!(
                            "Task #{} failed: {}",
                            row.task_id,
                            truncate_chars(&error, 500)
                        ),
                        now,
                        row.origin_hero_build_id,
                        row.target_language,
                    )
                    .execute(&mut *tx)
                    .await?;
                    if changed.rows_affected() > 0 {
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

        let reset = sqlx::query!(
            r#"
            UPDATE tierlist.hero_build_clones hbc
               SET status = 'pending',
                   status_info = $1,
                   attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END,
                   updated_at = $2
             WHERE status = 'processing'
               AND last_attempt_at < $3
               AND last_attempt_at IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1
                     FROM steam.steam_tasks t
                    WHERE t.type = 'BUILD_PUBLISH'
                      AND t.payload ? 'origin_hero_build_id'
                      AND t.payload->>'origin_hero_build_id' ~ '^-?[0-9]+$'
                      AND (t.payload->>'origin_hero_build_id')::BIGINT = hbc.origin_hero_build_id
                      AND t.status IN ('DONE', 'FAILED')
               )
            "#,
            STATUS_INFO_RESET_STALE,
            now,
            stale_threshold,
        )
        .execute(&mut *tx)
        .await?;
        stats.reset_stale = rows_affected_to_usize(reset.rows_affected())?;

        tx.commit().await?;
        Ok(stats)
    }
}

async fn upsert_kv_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO bot.kv_store(ns, k, v)
        VALUES('build_publisher', $1, $2)
        ON CONFLICT(ns, k) DO UPDATE SET v = EXCLUDED.v
        "#,
        key,
        value,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub fn spawn(pool: PgPool) -> Vec<tokio::task::JoinHandle<()>> {
    let publisher = Arc::new(BuildPublisher::new(
        pool,
        BuildPublisherConfig::production(),
    ));
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
    #![allow(clippy::too_many_arguments)]

    use super::*;
    use dl_central_db::TestDb;
    use serde_json::Value;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn test_db() -> Result<(TestDb, PgPool), Box<dyn std::error::Error + Send + Sync>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool().clone();
        Ok((db, pool))
    }

    fn publisher(pool: PgPool) -> BuildPublisher {
        BuildPublisher::new(
            pool,
            BuildPublisherConfig {
                publish_interval_seconds: 10 * 60,
                monitor_interval_seconds: 2 * 60,
                max_attempts: 3,
                batch_size: 5,
            },
        )
    }

    async fn set_steam_state(pool: &PgPool, logged_on: bool, gc_ready: bool) -> anyhow::Result<()> {
        let payload = json!({
            "runtime": {
                "logged_on": logged_on,
                "deadlock_gc_ready": gc_ready,
            }
        })
        .to_string();
        sqlx::query!(
            r#"
            INSERT INTO bot.standalone_bot_state(bot, heartbeat, payload)
            VALUES('steam', now(), $1::text::jsonb)
            ON CONFLICT(bot) DO UPDATE
               SET payload = EXCLUDED.payload,
                   heartbeat = EXCLUDED.heartbeat,
                   updated_at = now()
            "#,
            payload,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_clone(
        pool: &PgPool,
        origin_hero_build_id: i64,
        hero_id: i64,
        author_account_id: i64,
        priority: Option<i64>,
        publish_ts: i64,
        created_at: i64,
        attempts: i64,
    ) -> anyhow::Result<()> {
        let priority = priority.map(i32::try_from).transpose()?;
        let published_at = timestamp_seconds(publish_ts)?;
        let created_at = timestamp_seconds(created_at)?;
        let attempts = i32::try_from(attempts)?;
        let name = format!("source-{origin_hero_build_id}");
        let target_name = format!("target-{origin_hero_build_id}");
        let target_description = format!("description-{origin_hero_build_id}");

        sqlx::query!(
            r#"
            INSERT INTO tierlist.watched_build_authors(author_account_id, priority)
            VALUES($1, $2)
            ON CONFLICT(author_account_id) DO UPDATE
               SET priority = EXCLUDED.priority
            "#,
            author_account_id,
            priority,
        )
        .execute(pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.hero_build_sources(
                hero_build_id, author_account_id, hero_id, language, version, name, published_at
            )
            VALUES($1, $2, $3, 1, 1, $4, $5)
            "#,
            origin_hero_build_id,
            author_account_id,
            hero_id,
            name,
            published_at,
        )
        .execute(pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.hero_build_clones(
                origin_hero_build_id, hero_id, target_language, target_name,
                target_description, created_at, attempts
            )
            VALUES($1, $2, 2, $3, $4, $5, $6)
            "#,
            origin_hero_build_id,
            hero_id,
            target_name,
            target_description,
            created_at,
            attempts,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn task_payloads(pool: &PgPool) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query!(
            r#"
            SELECT payload::text AS "payload!"
              FROM steam.steam_tasks
             ORDER BY id
            "#
        )
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|row| row.payload).collect())
    }

    async fn clone_task_rows(
        pool: &PgPool,
    ) -> anyhow::Result<Vec<(i64, String, i32, Option<String>)>> {
        let rows = sqlx::query!(
            r#"
            SELECT c.origin_hero_build_id,
                   c.status,
                   c.attempts AS "attempts!",
                   t.payload::text AS "payload?"
              FROM tierlist.hero_build_clones c
              LEFT JOIN steam.steam_tasks t
                     ON t.payload ? 'origin_hero_build_id'
                    AND t.payload->>'origin_hero_build_id' ~ '^-?[0-9]+$'
                    AND (t.payload->>'origin_hero_build_id')::BIGINT = c.origin_hero_build_id
             ORDER BY c.origin_hero_build_id
            "#
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.origin_hero_build_id,
                    row.status,
                    row.attempts,
                    row.payload,
                )
            })
            .collect())
    }

    #[tokio::test]
    async fn process_queue_uses_ranked_top_three_then_processes_oldest_valid_tasks() -> TestResult {
        let (_db, pool) = test_db().await?;
        set_steam_state(&pool, true, true).await?;
        insert_clone(&pool, 100, 1, 10, Some(0), 100, 50, 0).await?;
        insert_clone(&pool, 101, 1, 11, Some(0), 200, 40, 0).await?;
        insert_clone(&pool, 102, 1, 12, Some(1), 300, 10, 0).await?;
        insert_clone(&pool, 103, 1, 13, Some(1), 100, 20, 0).await?;
        insert_clone(&pool, 104, 1, 14, None, 150, 30, 0).await?;

        let stats = publisher(pool.clone())
            .process_queue_at(1_000, "test")
            .await?;
        assert_eq!(stats.checked, 3);
        assert_eq!(stats.queued, 3);
        assert_eq!(stats.cancelled_excess, 2);

        let payloads = task_payloads(&pool).await?;
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
        let (_db, pool) = test_db().await?;
        set_steam_state(&pool, true, true).await?;
        insert_clone(&pool, 200, 1, 20, Some(0), 200, 10, 0).await?;
        insert_clone(&pool, 201, 1, 21, Some(0), 190, 20, 1).await?;
        insert_clone(&pool, 202, 1, 22, Some(0), 180, 30, 2).await?;
        insert_clone(&pool, 203, 2, 23, Some(0), 200, 40, 0).await?;
        insert_clone(&pool, 204, 2, 24, Some(0), 190, 50, 0).await?;
        insert_clone(&pool, 205, 2, 25, Some(0), 180, 60, 0).await?;
        insert_clone(&pool, 206, 3, 26, Some(0), 200, 70, 3).await?;

        let stats = publisher(pool.clone())
            .process_queue_at(2_000, "test")
            .await?;
        assert_eq!(stats.checked, 5);
        assert_eq!(stats.queued, 5);

        let rows = clone_task_rows(&pool).await?;

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
        let (_db, pool) = test_db().await?;
        set_steam_state(&pool, true, true).await?;
        for idx in 0..5 {
            insert_clone(
                &pool,
                300 + idx,
                7,
                30 + idx,
                Some(0),
                500 - idx,
                10 + idx,
                0,
            )
            .await?;
        }

        let stats = publisher(pool.clone())
            .process_queue_at(3_000, "test")
            .await?;
        assert_eq!(stats.cancelled_excess, 2);

        let cancelled = sqlx::query!(
            r#"
            SELECT origin_hero_build_id, status_info AS "status_info!"
              FROM tierlist.hero_build_clones
             WHERE status = 'cancelled'
             ORDER BY origin_hero_build_id
            "#
        )
        .fetch_all(&pool)
        .await?
        .into_iter()
        .map(|row| (row.origin_hero_build_id, row.status_info))
        .collect::<Vec<_>>();

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
            let (_db, pool) = test_db().await?;
            set_steam_state(&pool, logged_on, gc_ready).await?;
            insert_clone(&pool, 350, 1, 35, Some(0), 100, 10, 0).await?;

            let stats = publisher(pool.clone())
                .process_queue_at(4_000, "test")
                .await?;
            assert_eq!(stats.skipped, 1);
            assert_eq!(stats.checked, 0);
            assert_eq!(stats.queued, 0);
            assert_eq!(stats.cancelled_excess, 0);

            let task_count = sqlx::query!(r#"SELECT COUNT(*) AS "count!" FROM steam.steam_tasks"#)
                .fetch_one(&pool)
                .await?
                .count;
            let status = sqlx::query!(
                r#"
                SELECT status
                  FROM tierlist.hero_build_clones
                 WHERE origin_hero_build_id = 350
                "#
            )
            .fetch_one(&pool)
            .await?
            .status;
            assert_eq!(task_count, 0);
            assert_eq!(status, "pending");
        }

        Ok(())
    }

    #[tokio::test]
    async fn monitor_resets_only_processing_clones_older_than_thirty_minutes() -> TestResult {
        let (_db, pool) = test_db().await?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.hero_build_clones(
                origin_hero_build_id, hero_id, target_language, status,
                created_at, last_attempt_at, attempts
            ) VALUES
                (400, 1, 2, 'processing', $1, $2, 2),
                (401, 1, 2, 'processing', $1, $3, 2),
                (402, 1, 2, 'processing', $1, NULL, 2),
                (403, 1, 2, 'pending', $1, $4, 2),
                (404, 1, 2, 'processing', $1, $4, 0)
            "#,
            timestamp_seconds(1)?,
            timestamp_seconds(1199)?,
            timestamp_seconds(1200)?,
            timestamp_seconds(1100)?,
        )
        .execute(&pool)
        .await?;

        let stats = publisher(pool.clone()).monitor_tasks_at(3_000).await?;
        assert_eq!(stats.reset_stale, 2);

        let rows = sqlx::query!(
            r#"
            SELECT origin_hero_build_id, status, status_info, attempts AS "attempts!"
              FROM tierlist.hero_build_clones
             ORDER BY origin_hero_build_id
            "#
        )
        .fetch_all(&pool)
        .await?
        .into_iter()
        .map(|row| {
            (
                row.origin_hero_build_id,
                row.status,
                row.status_info,
                row.attempts,
            )
        })
        .collect::<Vec<_>>();

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
        let (_db, pool) = test_db().await?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.hero_build_clones(
                origin_hero_build_id, hero_id, target_language, status,
                created_at, updated_at, last_attempt_at, attempts
            ) VALUES
                (500, 1, 2, 'processing', $1, $1, $2, 1),
                (501, 1, 2, 'processing', $1, $1, $2, 1)
            "#,
            timestamp_seconds(1)?,
            timestamp_seconds(9500)?,
        )
        .execute(&pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO steam.steam_tasks(id, type, payload, status, result, finished_at)
            VALUES(900, 'BUILD_PUBLISH', $1::text::jsonb, 'DONE', $2::text::jsonb, $3)
            "#,
            json!({"origin_hero_build_id": 500}).to_string(),
            json!({"response": {"hero_build_id": 7000, "version": 12}}).to_string(),
            timestamp_seconds(9900)?,
        )
        .execute(&pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO steam.steam_tasks(id, type, payload, status, error, finished_at)
            VALUES(901, 'BUILD_PUBLISH', $1::text::jsonb, 'FAILED', 'upload failed hard', $2)
            "#,
            json!({"origin_hero_build_id": 501}).to_string(),
            timestamp_seconds(9901)?,
        )
        .execute(&pool)
        .await?;

        let stats = publisher(pool.clone()).monitor_tasks_at(10_000).await?;
        assert_eq!(stats.checked, 2);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.reset_stale, 0);

        let rows = sqlx::query!(
            r#"
            SELECT origin_hero_build_id, status, uploaded_build_id,
                   uploaded_version, status_info, updated_at AS "updated_at!"
              FROM tierlist.hero_build_clones
             ORDER BY origin_hero_build_id
            "#
        )
        .fetch_all(&pool)
        .await?
        .into_iter()
        .map(|row| {
            (
                row.origin_hero_build_id,
                row.status,
                row.uploaded_build_id,
                row.uploaded_version,
                row.status_info,
                row.updated_at,
            )
        })
        .collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![
                (
                    500,
                    "uploaded".to_string(),
                    Some(7000),
                    Some(12),
                    Some("Published as build #7000 v12".to_string()),
                    timestamp_seconds(10_000)?,
                ),
                (
                    501,
                    "failed".to_string(),
                    None,
                    None,
                    Some("Task #901 failed: upload failed hard".to_string()),
                    timestamp_seconds(10_000)?,
                ),
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn monitor_consumes_finished_stale_task_before_resetting_processing_clone() -> TestResult
    {
        let (_db, pool) = test_db().await?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.hero_build_clones(
                origin_hero_build_id, hero_id, target_language, status,
                created_at, updated_at, last_attempt_at, attempts
            ) VALUES(600, 1, 2, 'processing', $1, $1, $2, 2)
            "#,
            timestamp_seconds(1)?,
            timestamp_seconds(1000)?,
        )
        .execute(&pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO steam.steam_tasks(id, type, payload, status, result, finished_at)
            VALUES(910, 'BUILD_PUBLISH', $1::text::jsonb, 'DONE', $2::text::jsonb, $3)
            "#,
            json!({"origin_hero_build_id": 600}).to_string(),
            json!({"response": {"hero_build_id": 7600, "version": 3}}).to_string(),
            timestamp_seconds(2500)?,
        )
        .execute(&pool)
        .await?;

        let stats = publisher(pool.clone()).monitor_tasks_at(3_000).await?;
        assert_eq!(stats.checked, 1);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.reset_stale, 0);

        let row = sqlx::query!(
            r#"
            SELECT status, attempts AS "attempts!", uploaded_build_id, uploaded_version, status_info
              FROM tierlist.hero_build_clones
             WHERE origin_hero_build_id = 600
            "#
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            (
                row.status,
                row.attempts,
                row.uploaded_build_id,
                row.uploaded_version,
                row.status_info,
            ),
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
        let (_db, pool) = test_db().await?;
        sqlx::query!(
            r#"
            INSERT INTO tierlist.hero_build_clones(
                origin_hero_build_id, hero_id, target_language, status,
                created_at, updated_at, last_attempt_at, attempts
            ) VALUES(610, 1, 2, 'processing', $1, $1, $2, 1)
            "#,
            timestamp_seconds(1)?,
            timestamp_seconds(2500)?,
        )
        .execute(&pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO steam.steam_tasks(id, type, payload, status, result, finished_at)
            VALUES(920, 'BUILD_PUBLISH', $1::text::jsonb, 'DONE', $2::text::jsonb, $3)
            "#,
            json!({"origin_hero_build_id": 610}).to_string(),
            json!({"response": {"hero_build_id": 7610, "version": 1}}).to_string(),
            timestamp_seconds(2900)?,
        )
        .execute(&pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO steam.steam_tasks(id, type, payload, status, result, finished_at)
            VALUES(921, 'BUILD_PUBLISH', $1::text::jsonb, 'DONE', $2::text::jsonb, $3)
            "#,
            json!({"origin_hero_build_id": 610}).to_string(),
            json!({"response": {"hero_build_id": 7611, "version": 2}}).to_string(),
            timestamp_seconds(2950)?,
        )
        .execute(&pool)
        .await?;

        let stats = publisher(pool.clone()).monitor_tasks_at(3_000).await?;
        assert_eq!(stats.checked, 2);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.reset_stale, 0);

        let row = sqlx::query!(
            r#"
            SELECT status, uploaded_build_id, uploaded_version, status_info
              FROM tierlist.hero_build_clones
             WHERE origin_hero_build_id = 610
            "#
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            (
                row.status,
                row.uploaded_build_id,
                row.uploaded_version,
                row.status_info,
            ),
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
