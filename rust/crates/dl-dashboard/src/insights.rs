//! Server-Insights nutzt bewusst den kompakten Journey-Pfad ab 2026-07.
//! Alt-Historie kommt hier ueber CSV-Importe; `/api/voice-history` und
//! `/api/server-stats` bleiben fuer historische Totale auf `voice_session_log`.

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Multipart, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use crate::db::DashboardDbResult;
use crate::web::{err_text, ok_json, DashboardApp};

const DEFAULT_DAYS: i64 = 56;
const IMPORT_FILE_LIMIT: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interval {
    Daily,
    Weekly,
}

#[derive(Debug, Clone)]
struct InsightQuery {
    interval: Interval,
    from: NaiveDate,
    to: NaiveDate,
    guild_id: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct Period {
    start: NaiveDate,
    end: NaiveDate,
}

#[derive(Debug, Clone)]
struct ImportSpec {
    kind: &'static str,
    date_col: usize,
    dimension_col: Option<usize>,
    value_cols: Vec<usize>,
}

fn monday(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

fn parse_date(raw: &str) -> Option<NaiveDate> {
    let raw = raw.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(dt.date_naive());
    }
    // Nach Bytes schneiden, nicht nach Zeichen: `get` liefert bei einer
    // Mehrbyte-Grenze None statt zu paniken. Der Text kommt aus fremden CSVs.
    if let Some(head) = raw.get(..10) {
        if let Ok(date) = NaiveDate::parse_from_str(head, "%Y-%m-%d") {
            return Some(date);
        }
    }
    ["%Y-%m-%d", "%Y/%m/%d", "%d.%m.%Y"]
        .iter()
        .find_map(|fmt| NaiveDate::parse_from_str(raw, fmt).ok())
}

fn period_expr(column: &str, interval: Interval) -> String {
    match interval {
        Interval::Daily => format!("({column} AT TIME ZONE 'UTC')::date"),
        Interval::Weekly => format!("date_trunc('week', {column} AT TIME ZONE 'UTC')::date"),
    }
}

fn day_period_expr(column: &str, interval: Interval) -> String {
    match interval {
        Interval::Daily => column.to_string(),
        Interval::Weekly => format!("date_trunc('week', {column}::timestamp)::date"),
    }
}

fn parse_query(params: HashMap<String, String>) -> Result<InsightQuery, Response> {
    let interval = match params.get("interval").map(|s| s.trim().to_lowercase()) {
        None => Interval::Weekly,
        Some(s) if s == "weekly" => Interval::Weekly,
        Some(s) if s == "daily" => Interval::Daily,
        _ => return Err(err_text(400, "interval must be weekly or daily")),
    };
    let today = Utc::now().date_naive();
    let default_to = match interval {
        Interval::Daily => today,
        Interval::Weekly => monday(today),
    };
    let from = match params.get("from").and_then(|s| parse_date(s)) {
        Some(date) => date,
        None => default_to - Duration::days(DEFAULT_DAYS),
    };
    let to = match params.get("to").and_then(|s| parse_date(s)) {
        Some(date) => date + Duration::days(1),
        None => default_to,
    };
    if from >= to {
        return Err(err_text(400, "from must be before to"));
    }
    let guild_id = match params
        .get("guild_id")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => Some(
            raw.parse::<i64>()
                .map_err(|_| err_text(400, "guild_id must be an integer"))?,
        ),
    };
    Ok(InsightQuery {
        interval,
        from,
        to,
        guild_id,
    })
}

fn periods(query: &InsightQuery) -> Vec<Period> {
    let mut out = Vec::new();
    let mut start = match query.interval {
        Interval::Daily => query.from,
        Interval::Weekly => monday(query.from),
    };
    while start < query.to {
        let end = match query.interval {
            Interval::Daily => start + Duration::days(1),
            Interval::Weekly => start + Duration::days(7),
        };
        if end <= query.to && end > query.from {
            out.push(Period { start, end });
        }
        start = end;
    }
    out
}

fn current_and_previous(query: &InsightQuery) -> Option<(Period, Period)> {
    let all = periods(query);
    let current = *all.last()?;
    let previous = Period {
        start: current.start - (current.end - current.start),
        end: current.start,
    };
    Some((current, previous))
}

fn pct_change(current: Option<f64>, previous: Option<f64>) -> Value {
    match (current, previous) {
        (Some(c), Some(p)) if p != 0.0 => json!(((c - p) / p) * 100.0),
        _ => Value::Null,
    }
}

async fn imported(pool: &PgPool, query: &InsightQuery) -> DashboardDbResult<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT import_kind, period_start::text AS period_start, dimension, value::float8 AS value,
               imported_at
          FROM activity.insights_imports
         WHERE ($1::BIGINT IS NULL OR guild_id = $1)
           AND period_start >= $2
           AND period_start < $3
         ORDER BY import_kind, period_start, dimension
        "#,
    )
    .bind(query.guild_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "import_kind": row.get::<String, _>("import_kind"),
                "period_start": row.get::<String, _>("period_start"),
                "dimension": row.get::<String, _>("dimension"),
                "value": row.get::<f64, _>("value"),
                "imported_at": row.get::<DateTime<Utc>, _>("imported_at").to_rfc3339(),
            })
        })
        .collect())
}

async fn with_imported(pool: &PgPool, query: &InsightQuery, live: Value) -> Response {
    match imported(pool, query).await {
        Ok(rows) => ok_json(json!({ "live": live, "imported": rows })),
        Err(err) => {
            tracing::error!(%err, "Insights-Importdaten nicht lesbar");
            err_text(500, "Discord-Zahlen konnten nicht geladen werden")
        }
    }
}

async fn scalar_i64(
    pool: &PgPool,
    sql: &str,
    period: Period,
    guild: Option<i64>,
) -> DashboardDbResult<i64> {
    let row = sqlx::query(sql)
        .bind(guild)
        .bind(period.start)
        .bind(period.end)
        .fetch_one(pool)
        .await?;
    Ok(row.try_get::<i64, _>("value").unwrap_or(0))
}

async fn visitors(
    pool: &PgPool,
    period: Period,
    guild: Option<i64>,
) -> DashboardDbResult<Option<i64>> {
    let row = sqlx::query(
        r#"
        WITH raw AS (
            SELECT user_id
              FROM activity.presence_daily_seen
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND day >= $2
               AND day < $3
        ),
        agg AS (
            SELECT COALESCE(SUM(distinct_user_count), 0)::BIGINT AS daily_users
              FROM activity.presence_daily_aggregates
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND day >= $2
               AND day < $3
        )
        SELECT COUNT(*)::BIGINT AS raw_rows,
               COUNT(DISTINCT user_id)::BIGINT AS raw_users,
               (SELECT daily_users FROM agg) AS agg_users
          FROM raw
        "#,
    )
    .bind(guild)
    .bind(period.start)
    .bind(period.end)
    .fetch_one(pool)
    .await?;
    let raw_rows: i64 = row.get("raw_rows");
    let raw_users: i64 = row.get("raw_users");
    let agg_users: i64 = row.get("agg_users");
    if raw_rows == 0 && agg_users == 0 {
        Ok(None)
    } else {
        Ok(Some(raw_users + agg_users))
    }
}

async fn week1_retention_latest(
    pool: &PgPool,
    guild: Option<i64>,
) -> DashboardDbResult<Option<f64>> {
    let today_week = monday(Utc::now().date_naive());
    let row = sqlx::query(
        r#"
        WITH cohorts AS (
            SELECT date_trunc('week', occurred_at AT TIME ZONE 'UTC')::date AS cohort_week,
                   guild_id,
                   user_id
              FROM activity.member_events
             WHERE event_type = 'join'
               AND ($1::BIGINT IS NULL OR guild_id = $1)
               AND date_trunc('week', occurred_at AT TIME ZONE 'UTC')::date + interval '14 days' <= $2::date
        ),
        latest AS (
            SELECT MAX(cohort_week) AS cohort_week FROM cohorts
        ),
        activity_users AS (
            SELECT guild_id, user_id, (occurred_at AT TIME ZONE 'UTC')::date AS day FROM activity.message_metadata_events
            UNION ALL
            SELECT guild_id, user_id, (occurred_at AT TIME ZONE 'UTC')::date AS day FROM activity.voice_metadata_events
            UNION ALL
            SELECT guild_id, user_id, (occurred_at AT TIME ZONE 'UTC')::date AS day FROM activity.interaction_events
            UNION ALL
            SELECT guild_id, user_id, day FROM activity.presence_daily_seen
        )
        SELECT COUNT(*)::BIGINT AS joined,
               COUNT(*) FILTER (
                   WHERE EXISTS (
                       SELECT 1 FROM activity_users au
                        WHERE au.guild_id = c.guild_id
                          AND au.user_id = c.user_id
                          AND au.day >= c.cohort_week + 7
                          AND au.day < c.cohort_week + 14
                   )
               )::BIGINT AS retained
          FROM cohorts c
          JOIN latest l ON l.cohort_week = c.cohort_week
        "#,
    )
    .bind(guild)
    .bind(today_week)
    .fetch_one(pool)
    .await?;
    let joined: i64 = row.get("joined");
    let retained: i64 = row.get("retained");
    Ok((joined > 0).then_some(retained as f64 / joined as f64))
}

pub async fn overview(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let Some((current, previous)) = current_and_previous(&query) else {
        return err_text(400, "no completed period in range");
    };
    let read: DashboardDbResult<Value> = async {
        let joins_sql = "SELECT COUNT(*)::BIGINT AS value FROM activity.member_events WHERE event_type = 'join' AND ($1::BIGINT IS NULL OR guild_id = $1) AND occurred_at >= $2 AND occurred_at < $3";
        let new_members = scalar_i64(app.pool(), joins_sql, current, query.guild_id).await?;
        let prev_members = scalar_i64(app.pool(), joins_sql, previous, query.guild_id).await?;
        let contributors_sql = r#"
            SELECT COUNT(*)::BIGINT AS value
              FROM activity.journey_user_state
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND (
                    SELECT MIN(ts)
                      FROM (VALUES (first_message_at), (first_voice_at), (first_interaction_at)) v(ts)
                     WHERE ts IS NOT NULL
               ) >= $2
               AND (
                    SELECT MIN(ts)
                      FROM (VALUES (first_message_at), (first_voice_at), (first_interaction_at)) v(ts)
                     WHERE ts IS NOT NULL
               ) < $3
        "#;
        let new_contributors =
            scalar_i64(app.pool(), contributors_sql, current, query.guild_id).await?;
        let prev_new_contributors =
            scalar_i64(app.pool(), contributors_sql, previous, query.guild_id).await?;
        let contributors = contributor_count(app.pool(), current, query.guild_id).await?;
        let prev_contributors = contributor_count(app.pool(), previous, query.guild_id).await?;
        let messages = message_total(app.pool(), current, query.guild_id, query.interval).await?;
        let prev_messages =
            message_total(app.pool(), previous, query.guild_id, query.interval).await?;
        let voice_minutes_total =
            voice_minutes(app.pool(), current, query.guild_id, query.interval).await?;
        let prev_voice_minutes_total =
            voice_minutes(app.pool(), previous, query.guild_id, query.interval).await?;
        let visitors_total = visitors(app.pool(), current, query.guild_id).await?;
        let prev_visitors_total = visitors(app.pool(), previous, query.guild_id).await?;
        let retention = week1_retention_latest(app.pool(), query.guild_id).await?;

        let card = |key: &str, value: Option<f64>, prev: Option<f64>, basis: &str| {
            json!({
                "key": key,
                "value": value,
                "previous": prev,
                "change_pct": pct_change(value, prev),
                "basis": basis,
            })
        };
        Ok(json!({
            "period": { "start": current.start.to_string(), "end": current.end.to_string(), "interval": interval_name(query.interval) },
            "cards": [
                card("new_members", Some(new_members as f64), Some(prev_members as f64), "member_events"),
                card("new_contributors", Some(new_contributors as f64), Some(prev_new_contributors as f64), "journey_user_state_first_activity"),
                card("week1_retention", retention, Value::Null.as_f64(), "discord_weekly_cohort_latest_complete"),
                card("visitors", visitors_total.map(|v| v as f64), prev_visitors_total.map(|v| v as f64), "presence_daily_seen"),
                card("contributors", Some(contributors as f64), Some(prev_contributors as f64), "raw_user_events_180d"),
                card("messages_total", Some(messages as f64), Some(prev_messages as f64), "message_metadata_events_plus_daily_aggregates"),
                card("voice_minutes_total", Some(voice_minutes_total), Some(prev_voice_minutes_total), "voice_metadata_events_plus_voice_daily_aggregates"),
            ]
        }))
    }
    .await;
    match read {
        Ok(live) => {
            let imports = async {
                let rows = imported(app.pool(), &query).await?;
                let status = sqlx::query_scalar::<_, Value>(
                    "SELECT jsonb_build_object(
                        'last_import_at', max(imported_at),
                        'latest_period', max(period_start),
                        'rows', count(*),
                        'kinds', count(DISTINCT import_kind))
                     FROM activity.insights_imports
                     WHERE ($1::BIGINT IS NULL OR guild_id = $1)",
                )
                .bind(query.guild_id)
                .fetch_one(app.pool())
                .await?;
                Ok::<_, crate::db::DashboardDbError>((rows, status))
            }
            .await;
            match imports {
                Ok((rows, status)) => ok_json(json!({
                    "live": live, "imported": rows, "import_status": status,
                })),
                Err(err) => {
                    tracing::error!(%err, "Insights-Importdaten nicht lesbar");
                    err_text(500, "Discord-Zahlen konnten nicht geladen werden")
                }
            }
        }
        Err(err) => {
            tracing::error!(%err, "insights overview fehlgeschlagen");
            err_text(500, "Insights overview unavailable")
        }
    }
}

fn interval_name(interval: Interval) -> &'static str {
    match interval {
        Interval::Daily => "daily",
        Interval::Weekly => "weekly",
    }
}

async fn message_total(
    pool: &PgPool,
    period: Period,
    guild: Option<i64>,
    interval: Interval,
) -> DashboardDbResult<i64> {
    let raw = scalar_i64(
        pool,
        "SELECT COUNT(*)::BIGINT AS value FROM activity.message_metadata_events WHERE ($1::BIGINT IS NULL OR guild_id = $1) AND occurred_at >= $2 AND occurred_at < $3",
        period,
        guild,
    )
    .await?;
    let expr = day_period_expr("day", interval);
    let sql = format!(
        "SELECT COALESCE(SUM(message_count), 0)::BIGINT AS value FROM activity.message_daily_aggregates WHERE ($1::BIGINT IS NULL OR guild_id = $1) AND {expr} >= $2 AND {expr} < $3"
    );
    Ok(raw + scalar_i64(pool, &sql, period, guild).await?)
}

async fn voice_minutes(
    pool: &PgPool,
    period: Period,
    guild: Option<i64>,
    interval: Interval,
) -> DashboardDbResult<f64> {
    let raw = scalar_i64(
        pool,
        "SELECT COALESCE(SUM(duration_seconds), 0)::BIGINT AS value FROM activity.voice_metadata_events WHERE ($1::BIGINT IS NULL OR guild_id = $1) AND occurred_at >= $2 AND occurred_at < $3",
        period,
        guild,
    )
    .await?;
    let expr = day_period_expr("day", interval);
    let sql = format!(
        "SELECT COALESCE(SUM(total_duration_seconds), 0)::BIGINT AS value FROM activity.voice_daily_aggregates WHERE ($1::BIGINT IS NULL OR guild_id = $1) AND {expr} >= $2 AND {expr} < $3"
    );
    Ok((raw + scalar_i64(pool, &sql, period, guild).await?) as f64 / 60.0)
}

async fn contributor_count(
    pool: &PgPool,
    period: Period,
    guild: Option<i64>,
) -> DashboardDbResult<i64> {
    let row = sqlx::query(
        r#"
        WITH msg AS (
            SELECT user_id
              FROM activity.message_metadata_events
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND occurred_at >= $2
               AND occurred_at < $3
             GROUP BY user_id
            HAVING COUNT(*) >= 3
        ),
        voice AS (
            SELECT DISTINCT user_id
              FROM activity.voice_metadata_events
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND occurred_at >= $2
               AND occurred_at < $3
        )
        SELECT COUNT(DISTINCT user_id)::BIGINT AS value
          FROM (
              SELECT user_id FROM msg
              UNION ALL
              SELECT user_id FROM voice
          ) users
        "#,
    )
    .bind(guild)
    .bind(period.start)
    .bind(period.end)
    .fetch_one(pool)
    .await?;
    Ok(row.get("value"))
}

pub async fn growth(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let read = growth_live(app.pool(), &query).await;
    match read {
        Ok(live) => with_imported(app.pool(), &query, live).await,
        Err(err) => {
            tracing::error!(%err, "insights growth fehlgeschlagen");
            err_text(500, "Insights growth unavailable")
        }
    }
}

async fn growth_live(pool: &PgPool, query: &InsightQuery) -> DashboardDbResult<Value> {
    let expr = period_expr("occurred_at", query.interval);
    let join_sql = format!(
        "SELECT {expr} AS period, COALESCE(metadata->>'join_source_bucket', 'unknown') AS bucket, COUNT(*)::BIGINT AS count FROM activity.member_events WHERE event_type='join' AND ($1::BIGINT IS NULL OR guild_id=$1) AND occurred_at >= $2 AND occurred_at < $3 GROUP BY period, bucket ORDER BY period"
    );
    let mut by_period: BTreeMap<String, Value> = BTreeMap::new();
    for p in periods(query) {
        by_period.insert(
            p.start.to_string(),
            json!({"period_start": p.start.to_string(), "joins": {}, "leaves": {"lt_1_month": 0, "ge_1_month": 0}, "member_total": 0}),
        );
    }
    for row in sqlx::query(&join_sql)
        .bind(query.guild_id)
        .bind(query.from)
        .bind(query.to)
        .fetch_all(pool)
        .await?
    {
        let period: NaiveDate = row.get("period");
        let bucket: String = row.get("bucket");
        let count: i64 = row.get("count");
        if let Some(Value::Object(obj)) = by_period.get_mut(&period.to_string()) {
            if let Some(joins) = obj.get_mut("joins").and_then(Value::as_object_mut) {
                joins.insert(bucket, json!(count));
            }
        }
    }
    let leave_sql = format!(
        r#"
        SELECT {expr} AS period,
               CASE
                   WHEN j.joined_at IS NULL THEN 'unknown'
                   WHEN e.occurred_at - j.joined_at < interval '1 month' THEN 'lt_1_month'
                   ELSE 'ge_1_month'
               END AS bucket,
               COUNT(*)::BIGINT AS count
          FROM activity.member_events e
          LEFT JOIN LATERAL (
              SELECT occurred_at AS joined_at
                FROM activity.member_events j
               WHERE j.guild_id = e.guild_id
                 AND j.user_id = e.user_id
                 AND j.event_type = 'join'
                 AND j.occurred_at <= e.occurred_at
               ORDER BY j.occurred_at DESC
               LIMIT 1
          ) j ON TRUE
         WHERE e.event_type = 'leave'
           AND ($1::BIGINT IS NULL OR e.guild_id = $1)
           AND e.occurred_at >= $2
           AND e.occurred_at < $3
         GROUP BY period, bucket
        "#,
        expr = period_expr("e.occurred_at", query.interval)
    );
    for row in sqlx::query(&leave_sql)
        .bind(query.guild_id)
        .bind(query.from)
        .bind(query.to)
        .fetch_all(pool)
        .await?
    {
        let period: NaiveDate = row.get("period");
        let bucket: String = row.get("bucket");
        let count: i64 = row.get("count");
        if let Some(Value::Object(obj)) = by_period.get_mut(&period.to_string()) {
            if let Some(leaves) = obj.get_mut("leaves").and_then(Value::as_object_mut) {
                leaves.insert(bucket, json!(count));
            }
        }
    }
    let net_sql = format!(
        "SELECT {expr} AS period, COALESCE(SUM(CASE WHEN event_type='join' THEN 1 WHEN event_type='leave' THEN -1 ELSE 0 END), 0)::BIGINT AS net FROM activity.member_events WHERE event_type IN ('join','leave') AND ($1::BIGINT IS NULL OR guild_id=$1) AND occurred_at >= $2 AND occurred_at < $3 GROUP BY period",
        expr = period_expr("occurred_at", query.interval)
    );
    let mut nets = HashMap::new();
    for row in sqlx::query(&net_sql)
        .bind(query.guild_id)
        .bind(query.from)
        .bind(query.to)
        .fetch_all(pool)
        .await?
    {
        nets.insert(
            row.get::<NaiveDate, _>("period").to_string(),
            row.get::<i64, _>("net"),
        );
    }

    let directory_rows: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS value FROM activity.guild_member_directory WHERE ($1::BIGINT IS NULL OR guild_id = $1)",
    )
    .bind(query.guild_id)
    .fetch_one(pool)
    .await?
    .get("value");
    let member_total_basis = if directory_rows > 0 {
        let anchor: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS value FROM activity.guild_member_directory WHERE present AND NOT is_bot AND ($1::BIGINT IS NULL OR guild_id = $1)",
        )
        .bind(query.guild_id)
        .fetch_one(pool)
        .await?
        .get("value");
        let net_after_to: i64 = sqlx::query(
            "SELECT COALESCE(SUM(CASE WHEN event_type='join' THEN 1 WHEN event_type='leave' THEN -1 ELSE 0 END), 0)::BIGINT AS value FROM activity.member_events WHERE event_type IN ('join','leave') AND ($1::BIGINT IS NULL OR guild_id=$1) AND occurred_at >= $2::date AND occurred_at < now()",
        )
        .bind(query.guild_id)
        .bind(query.to)
        .fetch_one(pool)
        .await?
        .get("value");
        let mut total = anchor - net_after_to;
        for (key, value) in by_period.iter_mut().rev() {
            value["member_total"] = json!(total);
            total -= nets.get(key).copied().unwrap_or(0);
        }
        "guild_member_directory_present_backfilled_by_member_events_net"
    } else {
        let initial: i64 = sqlx::query(
            "SELECT COALESCE(SUM(CASE WHEN event_type='join' THEN 1 WHEN event_type='leave' THEN -1 ELSE 0 END), 0)::BIGINT AS value FROM activity.member_events WHERE ($1::BIGINT IS NULL OR guild_id=$1) AND occurred_at < $2",
        )
        .bind(query.guild_id)
        .bind(query.from)
        .fetch_one(pool)
        .await?
        .get("value");
        let mut total = initial;
        for (key, value) in by_period.iter_mut() {
            total += nets.get(key).copied().unwrap_or(0);
            value["member_total"] = json!(total);
        }
        "member_events_net_without_live_guild_anchor"
    };
    Ok(json!({
        "periods": by_period.into_values().collect::<Vec<_>>(),
        "member_total_basis": member_total_basis,
    }))
}

async fn audience_member_tenure(
    pool: &PgPool,
    guild: Option<i64>,
) -> DashboardDbResult<(Value, &'static str)> {
    let directory_rows: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS value FROM activity.guild_member_directory WHERE ($1::BIGINT IS NULL OR guild_id = $1)",
    )
    .bind(guild)
    .fetch_one(pool)
    .await?
    .get("value");

    if directory_rows > 0 {
        let rows = sqlx::query(
            r#"
            SELECT CASE
                    WHEN now() - joined_at < interval '1 month' THEN '<1_month'
                    WHEN now() - joined_at < interval '6 months' THEN '1_6_months'
                    WHEN now() - joined_at < interval '12 months' THEN '6_12_months'
                    ELSE '1_year_plus'
                   END AS bucket,
                   COUNT(*)::BIGINT AS count
              FROM activity.guild_member_directory
             WHERE present
               AND NOT is_bot
               AND joined_at IS NOT NULL
               AND ($1::BIGINT IS NULL OR guild_id = $1)
             GROUP BY bucket
            "#,
        )
        .bind(guild)
        .fetch_all(pool)
        .await?;
        return Ok((
            rows_to_bucket_counts(rows),
            "guild_member_directory_present",
        ));
    }

    let rows = sqlx::query(
        r#"
        WITH latest AS (
            SELECT DISTINCT ON (guild_id, user_id)
                   guild_id, user_id, event_type, occurred_at
              FROM activity.member_events
             WHERE event_type IN ('join', 'leave')
               AND ($1::BIGINT IS NULL OR guild_id = $1)
             ORDER BY guild_id, user_id, occurred_at DESC
        )
        SELECT CASE
                WHEN now() - occurred_at < interval '1 month' THEN '<1_month'
                WHEN now() - occurred_at < interval '6 months' THEN '1_6_months'
                WHEN now() - occurred_at < interval '12 months' THEN '6_12_months'
                ELSE '1_year_plus'
               END AS bucket,
               COUNT(*)::BIGINT AS count
          FROM latest
         WHERE event_type = 'join'
         GROUP BY bucket
        "#,
    )
    .bind(guild)
    .fetch_all(pool)
    .await?;
    Ok((
        rows_to_bucket_counts(rows),
        "member_events_latest_join_fallback",
    ))
}

pub async fn activation(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let expr = period_expr("joined_at", query.interval);
    let sql = format!(
        r#"
        SELECT {expr} AS period,
               COUNT(*)::BIGINT AS new_members,
               COUNT(*) FILTER (
                   WHERE (
                       first_message_at IS NOT NULL
                       AND (first_message_at AT TIME ZONE 'UTC')::date = (joined_at AT TIME ZONE 'UTC')::date
                   ) OR (
                       first_voice_at IS NOT NULL
                       AND (first_voice_at AT TIME ZONE 'UTC')::date = (joined_at AT TIME ZONE 'UTC')::date
                   )
               )::BIGINT AS activated
          FROM activity.journey_user_state
         WHERE joined_at IS NOT NULL
           AND ($1::BIGINT IS NULL OR guild_id = $1)
           AND joined_at >= $2
           AND joined_at < $3
         GROUP BY period
         ORDER BY period
        "#
    );
    let read: DashboardDbResult<Vec<Value>> = async {
        let mut rows = Vec::new();
        for row in sqlx::query(&sql)
            .bind(query.guild_id)
            .bind(query.from)
            .bind(query.to)
            .fetch_all(app.pool())
            .await?
        {
            let new_members: i64 = row.get("new_members");
            let activated: i64 = row.get("activated");
            rows.push(json!({
                "period_start": row.get::<NaiveDate, _>("period").to_string(),
                "new_members": new_members,
                "same_day_interaction_count": activated,
                "same_day_interaction_rate": if new_members > 0 { json!(activated as f64 / new_members as f64) } else { Value::Null },
            }));
        }
        Ok(rows)
    }
    .await;
    match read {
        Ok(live) => with_imported(app.pool(), &query, json!({ "periods": live })).await,
        Err(err) => {
            tracing::error!(%err, "insights activation fehlgeschlagen");
            err_text(500, "Insights activation unavailable")
        }
    }
}

pub async fn retention(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let mut query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    query.interval = Interval::Weekly;
    let today_week = monday(Utc::now().date_naive());
    let sql = r#"
        WITH cohorts AS (
            SELECT date_trunc('week', occurred_at AT TIME ZONE 'UTC')::date AS cohort_week,
                   guild_id,
                   user_id
              FROM activity.member_events
             WHERE event_type = 'join'
               AND ($1::BIGINT IS NULL OR guild_id = $1)
               AND occurred_at >= $2
               AND occurred_at < $3
               AND date_trunc('week', occurred_at AT TIME ZONE 'UTC')::date + interval '14 days' <= $4::date
        ),
        activity_users AS (
            SELECT guild_id, user_id, (occurred_at AT TIME ZONE 'UTC')::date AS day FROM activity.message_metadata_events
            UNION ALL
            SELECT guild_id, user_id, (occurred_at AT TIME ZONE 'UTC')::date AS day FROM activity.voice_metadata_events
            UNION ALL
            SELECT guild_id, user_id, (occurred_at AT TIME ZONE 'UTC')::date AS day FROM activity.interaction_events
            UNION ALL
            SELECT guild_id, user_id, day FROM activity.presence_daily_seen
        )
        SELECT c.cohort_week,
               COUNT(*)::BIGINT AS joined,
               COUNT(*) FILTER (
                   WHERE EXISTS (
                       SELECT 1 FROM activity_users au
                        WHERE au.guild_id = c.guild_id
                          AND au.user_id = c.user_id
                          AND au.day >= c.cohort_week + 7
                          AND au.day < c.cohort_week + 14
                   )
               )::BIGINT AS retained
          FROM cohorts c
         GROUP BY c.cohort_week
         ORDER BY c.cohort_week
    "#;
    let read: DashboardDbResult<Vec<Value>> = async {
        let mut out = Vec::new();
        for row in sqlx::query(sql)
            .bind(query.guild_id)
            .bind(query.from)
            .bind(query.to)
            .bind(today_week)
            .fetch_all(app.pool())
            .await?
        {
            let joined: i64 = row.get("joined");
            let retained: i64 = row.get("retained");
            out.push(json!({
                "cohort_week": row.get::<NaiveDate, _>("cohort_week").to_string(),
                "joined": joined,
                "retained": retained,
                "retention_rate": if joined > 0 { json!(retained as f64 / joined as f64) } else { Value::Null },
                "basis": "raw_message_voice_interaction_presence_events",
            }));
        }
        Ok(out)
    }
    .await;
    match read {
        Ok(live) => with_imported(app.pool(), &query, json!({ "cohorts": live })).await,
        Err(err) => {
            tracing::error!(%err, "insights retention fehlgeschlagen");
            err_text(500, "Insights retention unavailable")
        }
    }
}

pub async fn engagement(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let read: DashboardDbResult<Vec<Value>> = async {
        let mut out = Vec::new();
        for period in periods(&query) {
            let messages = message_total(app.pool(), period, query.guild_id, query.interval).await?;
            let contributors = contributor_count(app.pool(), period, query.guild_id).await?;
            let voice = voice_minutes(app.pool(), period, query.guild_id, query.interval).await?;
            out.push(json!({
                "period_start": period.start.to_string(),
                "visitors": visitors(app.pool(), period, query.guild_id).await?,
                "contributors": contributors,
                "basis": "contributors_from_message_metadata_events_and_voice_metadata_events_within_180d",
                "messages_total": messages,
                "avg_messages_per_contributor": if contributors > 0 { json!(messages as f64 / contributors as f64) } else { Value::Null },
                "voice_minutes_total": voice,
            }));
        }
        Ok(out)
    }
    .await;
    match read {
        Ok(live) => with_imported(app.pool(), &query, json!({ "periods": live })).await,
        Err(err) => {
            tracing::error!(%err, "insights engagement fehlgeschlagen");
            err_text(500, "Insights engagement unavailable")
        }
    }
}

pub async fn audience(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let read: DashboardDbResult<Value> = async {
        let (member_tenure, member_tenure_basis) =
            audience_member_tenure(app.pool(), query.guild_id).await?;
        let account_rows = sqlx::query(
            r#"
            SELECT CASE
                    WHEN occurred_at - account_created_at < interval '1 day' THEN '<1_day'
                    WHEN occurred_at - account_created_at < interval '1 month' THEN '<1_month'
                    ELSE '1_month_plus'
                   END AS bucket,
                   COUNT(*)::BIGINT AS count
              FROM activity.member_events
             WHERE event_type = 'join'
               AND account_created_at IS NOT NULL
               AND occurred_at >= now() - interval '28 days'
               AND ($1::BIGINT IS NULL OR guild_id = $1)
             GROUP BY bucket
            "#,
        )
        .bind(query.guild_id)
        .fetch_all(app.pool())
        .await?;
        Ok(json!({
            "member_tenure": member_tenure,
            "new_member_account_age_28d": rows_to_bucket_counts(account_rows),
            "basis": {
                "member_tenure": member_tenure_basis,
                "new_member_account_age_28d": "member_events_joins_28d"
            },
        }))
    }
    .await;
    match read {
        Ok(live) => with_imported(app.pool(), &query, live).await,
        Err(err) => {
            tracing::error!(%err, "insights audience fehlgeschlagen");
            err_text(500, "Insights audience unavailable")
        }
    }
}

fn rows_to_bucket_counts(rows: Vec<sqlx::postgres::PgRow>) -> Value {
    let mut map = serde_json::Map::new();
    for row in rows {
        map.insert(
            row.get::<String, _>("bucket"),
            json!(row.get::<i64, _>("count")),
        );
    }
    Value::Object(map)
}

pub async fn top_invites(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_query(params) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let read: DashboardDbResult<Vec<Value>> = async {
        let rows = sqlx::query(
            r#"
            SELECT metadata->>'invite_code' AS invite_code,
                   metadata->>'inviter_id' AS inviter_id,
                   metadata->>'inviter_name' AS inviter_label,
                   COUNT(*)::BIGINT AS joins
              FROM activity.member_events
             WHERE event_type = 'join'
               AND occurred_at >= now() - interval '28 days'
               AND metadata ? 'invite_code'
               AND ($1::BIGINT IS NULL OR guild_id = $1)
             GROUP BY invite_code, inviter_id, inviter_label
             ORDER BY joins DESC, invite_code
             LIMIT 50
            "#,
        )
        .bind(query.guild_id)
        .fetch_all(app.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({
                    "invite_code": row.get::<Option<String>, _>("invite_code"),
                    "inviter_id": row.get::<Option<String>, _>("inviter_id"),
                    "inviter_label": row.get::<Option<String>, _>("inviter_label"),
                    "joins": row.get::<i64, _>("joins"),
                })
            })
            .collect())
    }
    .await;
    match read {
        Ok(live) => with_imported(app.pool(), &query, json!({ "top_invites": live })).await,
        Err(err) => {
            tracing::error!(%err, "insights top-invites fehlgeschlagen");
            err_text(500, "Insights top invites unavailable")
        }
    }
}

pub async fn import(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, true).await {
        return resp;
    }
    let guild_id = match params
        .get("guild_id")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(raw) => match raw.parse::<i64>() {
            Ok(id) => id,
            Err(_) => return import_error_json(400, "Guild-ID muss eine Zahl sein", json!({})),
        },
        None => {
            return import_error_json(400, "Guild-ID fehlt (Query-Parameter guild_id)", json!({}))
        }
    };

    let mut files = 0usize;
    let mut imported_rows = 0usize;
    let mut results = Vec::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name().unwrap_or("upload.csv").to_string();
        let content_type = field.content_type().unwrap_or("").to_string();
        if !is_csv_upload(&content_type, &filename) {
            return import_error_json(
                415,
                "Datei ist keine CSV — bitte die Exporte aus dem Discord-Portal unverändert hochladen",
                json!({ "file": filename, "content_type": content_type }),
            );
        }
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                return import_error_json(
                    400,
                    "Upload konnte nicht gelesen werden — bitte erneut versuchen",
                    json!({ "file": filename }),
                )
            }
        };
        if bytes.len() > IMPORT_FILE_LIMIT {
            return import_error_json(
                413,
                "Datei ist größer als 5 MB — das ist kein Insights-Export",
                json!({ "file": filename }),
            );
        }
        let text =
            match std::str::from_utf8(&bytes) {
                Ok(text) => text,
                Err(_) => return import_error_json(
                    400,
                    "Datei ist kein UTF-8-Text — bitte den Original-Export unverändert hochladen",
                    json!({ "file": filename }),
                ),
            };
        let (kind, file_rows) = match import_csv_text(app.pool(), guild_id, text).await {
            Ok(result) => result,
            Err(err) if err.starts_with("Export-Typ nicht erkannt") => {
                return import_error_json(
                    400,
                    "Export-Typ nicht erkannt — die gefundenen Spalten stehen unten, wir ergänzen die Erkennung dann",
                    json!({ "file": filename, "detail": err }),
                )
            }
            Err(err) if err.contains("leer") => {
                return import_error_json(400, "CSV ist leer", json!({ "file": filename }))
            }
            Err(err) => {
                tracing::error!(%err, "Insights-Import fehlgeschlagen");
                return import_error_json(
                    500,
                    "Import konnte nicht gespeichert werden — Details stehen im Server-Log",
                    json!({ "file": filename }),
                );
            }
        };
        files += 1;
        imported_rows += file_rows;
        results.push(json!({ "file": filename, "import_kind": kind, "rows": file_rows }));
    }
    if files == 0 {
        return import_error_json(400, "Keine CSV-Dateien im Upload", json!({}));
    }
    ok_json(json!({ "files": files, "rows": imported_rows, "results": results }))
}

fn import_error_json(status: u16, message: &str, extra: Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        Json(json!({ "error": message, "details": extra })),
    )
        .into_response()
}

/// Importiert einen Discord-Insights-CSV-Text in `activity.insights_imports`.
pub async fn import_csv_text(
    pool: &PgPool,
    guild_id: i64,
    text: &str,
) -> Result<(&'static str, usize), String> {
    let rows = parse_csv(text)
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| "CSV ist leer oder nicht parsebar".to_string())?;
    let headers = rows[0].clone();
    let spec = detect_import(&headers)
        .ok_or_else(|| format!("Export-Typ nicht erkannt, Spalten: {}", headers.join(", ")))?;
    // Erst vollstaendig parsen, dann schreiben. Sonst kann ein Fehler mitten in
    // der Datei eine halb geleerte Tabelle hinterlassen.
    let mut parsed: Vec<(NaiveDate, String, f64)> = Vec::new();
    for row in rows.iter().skip(1) {
        if row.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let Some(period_start) = row.get(spec.date_col).and_then(|s| parse_date(s)) else {
            continue;
        };
        for &value_col in &spec.value_cols {
            let Some(value) = row.get(value_col).and_then(|s| parse_number(s)) else {
                continue;
            };
            let dimension = spec
                .dimension_col
                .and_then(|idx| row.get(idx))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    headers
                        .get(value_col)
                        .map(String::as_str)
                        .unwrap_or("value")
                });
            parsed.push((period_start, dimension.to_string(), value));
        }
    }

    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
    // Bei den Quellen-Exporten kann eine Quelle aus der Liste verschwinden, ein
    // reines Upsert liesse sie als Karteileiche stehen. Ersetzt wird deshalb der
    // Zeitraum, den diese Datei abdeckt, und nur der: aeltere Wochen sind
    // Historie, die kein Wochenlauf anfassen darf.
    if spec.kind == "joins_by_source" {
        if let (Some(von), Some(bis)) = (
            parsed.iter().map(|(date, _, _)| *date).min(),
            parsed.iter().map(|(date, _, _)| *date).max(),
        ) {
            sqlx::query(
                r#"
                DELETE FROM activity.insights_imports
                WHERE guild_id = $1
                  AND import_kind = $2
                  AND period_start BETWEEN $3 AND $4
                "#,
            )
            .bind(guild_id)
            .bind(spec.kind)
            .bind(von)
            .bind(bis)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        }
    }
    let file_rows = parsed.len();
    for (period_start, dimension, value) in &parsed {
        upsert_import(
            &mut tx,
            guild_id,
            spec.kind,
            *period_start,
            dimension,
            *value,
        )
        .await
        .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok((spec.kind, file_rows))
}

async fn upsert_import(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    guild_id: i64,
    kind: &str,
    period_start: NaiveDate,
    dimension: &str,
    value: f64,
) -> DashboardDbResult<()> {
    sqlx::query(
        r#"
        INSERT INTO activity.insights_imports(
            guild_id, import_kind, period_start, dimension, value, imported_at
        )
        VALUES($1, $2, $3, $4, $5::text::numeric, now())
        ON CONFLICT(guild_id, import_kind, period_start, dimension) DO UPDATE SET
            value = EXCLUDED.value,
            imported_at = EXCLUDED.imported_at
        "#,
    )
    .bind(guild_id)
    .bind(kind)
    .bind(period_start)
    .bind(dimension)
    .bind(value.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn normalize_header(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn detect_import(headers: &[String]) -> Option<ImportSpec> {
    let normalized = headers
        .iter()
        .map(|h| normalize_header(h))
        .collect::<Vec<_>>();
    let find = |needles: &[&str]| {
        normalized
            .iter()
            .position(|h| needles.iter().any(|needle| h.contains(needle)))
    };
    let date_col = find(&[
        "date",
        "day",
        "week",
        "period",
        "cohort",
        "interval",
        "timestamp",
    ])?;
    let value_cols = normalized
        .iter()
        .enumerate()
        .filter_map(|(idx, h)| {
            (idx != date_col
                && (h == "invites"
                    || [
                        "joins",
                        "leavers",
                        "leaves",
                        "retention",
                        "retained",
                        "activated",
                        "activation",
                        "visitors",
                        "contributors",
                        "communicators",
                        "communicated",
                        "messages",
                        "minutes",
                        "speaking",
                        "members",
                        "membership",
                        "opened",
                        "count",
                        "value",
                        "muters",
                        "readers",
                        "chatters",
                        "listeners",
                    ]
                    .iter()
                    .any(|needle| h.contains(*needle))))
            .then_some(idx)
        })
        .collect::<Vec<_>>();
    if value_cols.is_empty() {
        return None;
    }
    let dimension_col = find(&[
        "source",
        "referrer",
        "duration",
        "days_in",
        "invite",
        "code",
        "link",
        "bucket",
        "dimension",
        "channel",
        "domain",
    ])
    .filter(|idx| *idx != date_col && !value_cols.contains(idx));
    let joined = normalized.join("|");
    let kind = if (joined.contains("invite")
        && (joined.contains("code") || joined.contains("link")))
        || joined.contains("einladungslink")
    {
        "top_invites"
    } else if joined.contains("referrer") || joined.contains("referring") {
        "referrer"
    } else if joined.contains("retained")
        || joined.contains("retention")
        || joined.contains("cohort")
    {
        "retention"
    } else if joined.contains("opened")
        || (joined.contains("communicated")
            && joined.contains("new_members")
            && !joined.contains("visitor"))
        || joined.contains("activation")
        || joined.contains("activated")
    {
        "activation"
    } else if joined.contains("membership") && !joined.contains("new_members") {
        "membership"
    } else if joined.contains("listener") {
        "popular_voice"
    } else if joined.contains("reader") || joined.contains("chatter") {
        "popular_text"
    } else if joined.contains("visitor")
        || joined.contains("contributor")
        || joined.contains("communicator")
        || joined.contains("message")
        || joined.contains("speaking")
    {
        "engagement"
    } else if joined.contains("leave") || joined.contains("leaver") {
        "leavers"
    } else if joined.contains("muter") {
        "muters"
    } else if joined.contains("source") || joined.contains("join") {
        "joins_by_source"
    } else {
        return None;
    };
    Some(ImportSpec {
        kind,
        date_col,
        dimension_col,
        value_cols,
    })
}

fn parse_number(raw: &str) -> Option<f64> {
    raw.trim()
        .trim_end_matches('%')
        .replace(',', ".")
        .parse::<f64>()
        .ok()
}

fn is_csv_upload(content_type: &str, filename: &str) -> bool {
    let content_type = content_type.trim().to_ascii_lowercase();
    content_type.contains("csv")
        || content_type == "application/vnd.ms-excel"
        || filename.to_ascii_lowercase().ends_with(".csv")
}

fn parse_csv(input: &str) -> Option<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    for line in input.lines() {
        let mut row = Vec::new();
        let mut cell = String::new();
        let mut chars = line.chars().peekable();
        let mut quoted = false;
        while let Some(ch) = chars.next() {
            match ch {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => {
                    row.push(cell.trim().to_string());
                    cell.clear();
                }
                _ => cell.push(ch),
            }
        }
        if quoted {
            return None;
        }
        row.push(cell.trim().to_string());
        rows.push(row);
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekly_periods_use_completed_monday_weeks() {
        let query = InsightQuery {
            interval: Interval::Weekly,
            from: NaiveDate::from_ymd_opt(2026, 6, 1).expect("valid fixture date"),
            to: NaiveDate::from_ymd_opt(2026, 6, 22).expect("valid fixture date"),
            guild_id: None,
        };
        let got = periods(&query);
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0].start,
            NaiveDate::from_ymd_opt(2026, 6, 1).expect("valid fixture date")
        );
        assert_eq!(
            got[2].end,
            NaiveDate::from_ymd_opt(2026, 6, 22).expect("valid fixture date")
        );
    }

    #[test]
    fn parse_date_akzeptiert_discord_iso_stempel() {
        assert_eq!(
            parse_date("2026-03-31T00:00:00+00:00"),
            NaiveDate::from_ymd_opt(2026, 3, 31)
        );
        assert_eq!(
            parse_date("2026-08-17"),
            NaiveDate::from_ymd_opt(2026, 8, 17)
        );
    }

    #[test]
    fn parse_date_paniked_nicht_an_einer_mehrbyte_grenze() {
        // In "Zeitraum über 4 Wochen" beginnt das ü bei Byte 9 und reicht bis
        // Byte 10: ein Slice `[..10]` schneidet mitten hinein und reisst den
        // ganzen Import ab. Der Text kommt aus fremden CSVs, ist also beliebig.
        assert_eq!(parse_date("Zeitraum über 4 Wochen"), None);
        assert_eq!(parse_date("äöüäöüäöüäöü"), None);
        assert_eq!(parse_date(""), None);
        assert_eq!(parse_date("kurz"), None);
    }

    #[test]
    fn csv_parser_handles_quoted_commas() {
        let rows = parse_csv("Date,Source,Joins\n2026-06-01,\"Vanity, Link\",12\n")
            .expect("csv fixture parses");
        assert_eq!(rows[1][1], "Vanity, Link");
    }

    /// Der Wochenjob darf nur seinen eigenen Zeitraum ersetzen. Frueher loeschte
    /// er die komplette joins_by_source-Historie der Guild.
    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn joins_by_source_import_laesst_aeltere_wochen_stehen(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::db::test_pool().await?;
        let pool = db.pool();
        let guild = 7001i64;

        import_csv_text(pool, guild, "Week,Join Source,Joins\n2026-01-05,Vanity,5\n").await?;
        import_csv_text(
            pool,
            guild,
            "Week,Join Source,Joins\n2026-06-01,Vanity,12\n2026-06-01,Discovery,3\n",
        )
        .await?;

        let vorher = insights_rows(pool, guild).await?;
        assert_eq!(
            vorher,
            vec![
                ("2026-01-05".to_string(), "Vanity".to_string(), 5.0),
                ("2026-06-01".to_string(), "Discovery".to_string(), 3.0),
                ("2026-06-01".to_string(), "Vanity".to_string(), 12.0),
            ]
        );

        // Neuer Export derselben Woche, Discovery ist verschwunden: die Woche
        // wird ersetzt, die Januar-Zeile bleibt unberuehrt.
        import_csv_text(
            pool,
            guild,
            "Week,Join Source,Joins\n2026-06-01,Vanity,20\n",
        )
        .await?;
        let nachher = insights_rows(pool, guild).await?;
        assert_eq!(
            nachher,
            vec![
                ("2026-01-05".to_string(), "Vanity".to_string(), 5.0),
                ("2026-06-01".to_string(), "Vanity".to_string(), 20.0),
            ],
            "aeltere Wochen sind Historie und ueberleben den naechsten Import"
        );
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn importierte_reihen_liefern_zeitpunkt_und_nur_die_gewaehlte_guild(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::db::test_pool().await?;
        for (guild, value) in [(7001, 12), (7002, 99)] {
            import_csv_text(
                db.pool(),
                guild,
                &format!("Week,Join Source,Joins\n2026-06-01,Vanity,{value}\n"),
            )
            .await?;
        }
        let rows = imported(
            db.pool(),
            &InsightQuery {
                guild_id: Some(7001),
                interval: Interval::Weekly,
                from: NaiveDate::from_ymd_opt(2026, 6, 1).expect("gültige Testdaten"),
                to: NaiveDate::from_ymd_opt(2026, 6, 8).expect("gültige Testdaten"),
            },
        )
        .await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["value"], 12.0);
        assert!(!rows[0]["imported_at"]
            .as_str()
            .expect("gültige Testdaten")
            .is_empty());
        Ok(())
    }

    #[cfg(feature = "testing")]
    async fn insights_rows(
        pool: &PgPool,
        guild_id: i64,
    ) -> Result<Vec<(String, String, f64)>, sqlx::Error> {
        let rows: Vec<(NaiveDate, String, String)> = sqlx::query_as(
            "SELECT period_start, dimension, value::text
               FROM activity.insights_imports
              WHERE guild_id = $1 AND import_kind = 'joins_by_source'
              ORDER BY period_start, dimension",
        )
        .bind(guild_id)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(period, dimension, value)| {
                (
                    period.to_string(),
                    dimension,
                    value.parse::<f64>().unwrap_or_default(),
                )
            })
            .collect())
    }

    #[test]
    fn import_detection_recognizes_join_source_export() {
        let headers = vec![
            "Week".to_string(),
            "Join Source".to_string(),
            "Joins".to_string(),
        ];
        let spec = detect_import(&headers).expect("fixture signature is known");
        assert_eq!(spec.kind, "joins_by_source");
        assert_eq!(spec.dimension_col, Some(1));
        assert_eq!(spec.value_cols, vec![2]);
    }

    #[test]
    fn import_detection_erkennt_offizielle_discord_csvs() {
        let cases = [
            (
                "day_pt,new_members,pct_communicated,pct_opened_channels,interval_start_timestamp",
                "activation",
            ),
            (
                "interval_start_timestamp,visitors,pct_communicated",
                "engagement",
            ),
            (
                "day_pt,discovery_joins,invites,vanity_joins,other_joins,total_joins,interval_start_timestamp",
                "joins_by_source",
            ),
            (
                "day_pt,days_in_guild,leavers,interval_start_timestamp",
                "leavers",
            ),
            (
                "interval_start_timestamp,messages,messages_per_communicator",
                "engagement",
            ),
            (
                "day_pt,new_members,pct_retained,interval_start_timestamp",
                "retention",
            ),
            (
                "day_pt,total_membership,interval_start_timestamp",
                "membership",
            ),
        ];
        for (header_line, kind) in cases {
            let headers = header_line
                .split(',')
                .map(str::to_string)
                .collect::<Vec<_>>();
            let spec = detect_import(&headers)
                .unwrap_or_else(|| panic!("official header not recognized: {header_line}"));
            assert_eq!(spec.kind, kind, "header {header_line}");
            if kind == "joins_by_source" && headers.iter().any(|h| h == "invites") {
                assert_eq!(
                    spec.dimension_col, None,
                    "invites ist Wert, keine Dimension"
                );
                assert!(
                    spec.value_cols.iter().any(|&i| headers[i] == "invites"),
                    "Spalte invites muss importiert werden"
                );
            }
        }
    }

    #[test]
    fn csv_upload_erlaubt_browser_varianten() {
        assert!(is_csv_upload("text/csv; charset=utf-8", "export.bin"));
        assert!(is_csv_upload("application/vnd.ms-excel", "export.xls"));
        assert!(is_csv_upload("application/octet-stream", "Export.CSV"));
        assert!(!is_csv_upload("application/json", "export.json"));
    }
}
