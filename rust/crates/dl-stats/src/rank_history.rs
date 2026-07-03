//! Rank-History-API fuer das Aktivitaets-Dashboard.

use std::collections::HashMap;

#[cfg(test)]
use std::cmp::Ordering;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;

use crate::public::resolve_display_names;
use crate::timeutil::isoformat_utc;
use crate::{
    internal_error, parse_positive_int, require_session, session_user_id, SharedApp, StatsApp,
};

const MAIN_GUILD_ID: i64 = 1_289_721_245_281_292_288;
const MAX_RANK_HISTORY_DAYS: i64 = 3650;

type Params = Query<HashMap<String, String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RankVisibility {
    Private,
    Members,
    Public,
}

impl RankVisibility {
    fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Members => "members",
            Self::Public => "public",
        }
    }
}

impl LeaderboardSort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Climb => "climb",
            Self::Top => "top",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaderboardSort {
    Climb,
    Top,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RankSnapshot {
    badge_level: i32,
    rank_name: String,
    captured_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct RankEntry {
    badge_level: i32,
    rank_name: String,
    captured_at: String,
}

#[derive(Debug, Serialize)]
struct MeRankHistoryPayload {
    current: Option<RankEntry>,
    history: Vec<RankEntry>,
    visibility: String,
}

#[derive(Debug, Serialize)]
struct PublicRankHistoryPayload {
    current: RankEntry,
    history: Vec<RankEntry>,
}

#[derive(Debug)]
struct LeaderboardRow {
    user_id: i64,
    rank_name: String,
    badge_level: i32,
    delta: i32,
}

#[derive(Debug, Serialize)]
struct LeaderboardEntry {
    user_id: String,
    display_name: String,
    rank_name: String,
    badge_level: i32,
    delta: i32,
}

#[derive(Debug, Deserialize)]
pub struct VisibilityRequest {
    visibility: String,
}

impl From<RankSnapshot> for RankEntry {
    fn from(value: RankSnapshot) -> Self {
        Self {
            badge_level: value.badge_level,
            rank_name: value.rank_name,
            captured_at: isoformat_utc(value.captured_at),
        }
    }
}

fn db_error(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "Rank-History: DB-Fehler");
    internal_error()
}

fn invalid_visibility_response() -> Response {
    dl_webcore::envelope::error_code(StatusCode::BAD_REQUEST, "invalid_visibility")
}

fn invalid_sort_response() -> Response {
    dl_webcore::envelope::error_code(StatusCode::BAD_REQUEST, "invalid_sort")
}

fn not_found_response() -> Response {
    dl_webcore::envelope::error_code(StatusCode::NOT_FOUND, "not_found")
}

fn parse_visibility(raw: &str) -> Option<RankVisibility> {
    match raw {
        "private" => Some(RankVisibility::Private),
        "members" => Some(RankVisibility::Members),
        "public" => Some(RankVisibility::Public),
        _ => None,
    }
}

fn parse_sort(raw: Option<&String>) -> Result<LeaderboardSort, Response> {
    match raw.map(String::as_str).unwrap_or("climb") {
        "climb" => Ok(LeaderboardSort::Climb),
        "top" => Ok(LeaderboardSort::Top),
        _ => Err(invalid_sort_response()),
    }
}

fn parse_history_days(raw: Option<&String>) -> Result<i64, Response> {
    parse_rank_days(raw, 30)
}

fn parse_leaderboard_days(raw: Option<&String>) -> Result<i64, Response> {
    parse_rank_days(raw, 30)
}

fn parse_rank_days(raw: Option<&String>, default: i64) -> Result<i64, Response> {
    let invalid = || dl_webcore::envelope::error_code(StatusCode::BAD_REQUEST, "invalid_days");
    let value = match raw {
        None => default,
        Some(raw) => raw.trim().parse::<i64>().map_err(|_| invalid())?,
    };
    if value < 0 {
        return Err(invalid());
    }
    Ok(value.min(MAX_RANK_HISTORY_DAYS))
}

fn visibility_from_db(raw: Option<String>) -> RankVisibility {
    raw.as_deref()
        .and_then(parse_visibility)
        .unwrap_or(RankVisibility::Private)
}

fn visible_to_viewer(visibility: RankVisibility, viewer_is_member: bool) -> bool {
    match visibility {
        RankVisibility::Public => true,
        RankVisibility::Members => viewer_is_member,
        RankVisibility::Private => false,
    }
}

fn allowed_visibility_values(viewer_is_member: bool) -> Vec<String> {
    let mut values = vec![RankVisibility::Public.as_str().to_string()];
    if viewer_is_member {
        values.push(RankVisibility::Members.as_str().to_string());
    }
    values
}

fn optional_session_user_id(app: &StatsApp, headers: &HeaderMap) -> Option<i64> {
    let session = require_session(app, headers).ok()?;
    session_user_id(&session).ok()
}

fn parse_target_user_id(raw: &str) -> Option<i64> {
    raw.trim().parse::<i64>().ok().filter(|id| *id > 0)
}

#[cfg(test)]
fn current_rank_order(left: &RankSnapshot, right: &RankSnapshot) -> Ordering {
    left.captured_at
        .cmp(&right.captured_at)
        .then_with(|| left.badge_level.cmp(&right.badge_level))
}

#[cfg(test)]
fn oldest_rank_order(left: &RankSnapshot, right: &RankSnapshot) -> Ordering {
    left.captured_at
        .cmp(&right.captured_at)
        .then_with(|| right.badge_level.cmp(&left.badge_level))
}

/// Berechnet die Rank-Differenz. `days = 0` bedeutet All-Time:
/// Baseline ist der aelteste Eintrag, bei gleichem `captured_at` der hoechste
/// `badge_level`.
#[cfg(test)]
fn rank_delta(history: &[RankSnapshot], days: i64, now: DateTime<Utc>) -> Option<i32> {
    let current = history
        .iter()
        .max_by(|left, right| current_rank_order(left, right))?;
    let baseline = if days == 0 {
        history
            .iter()
            .min_by(|left, right| oldest_rank_order(left, right))
    } else {
        let cutoff = now - Duration::days(days);
        history
            .iter()
            .filter(|entry| entry.captured_at <= cutoff)
            .max_by(|left, right| current_rank_order(left, right))
            .or_else(|| {
                history
                    .iter()
                    .min_by(|left, right| oldest_rank_order(left, right))
            })
    }?;
    Some(current.badge_level - baseline.badge_level)
}

async fn current_rank(pool: &PgPool, user_id: i64) -> Result<Option<RankSnapshot>, sqlx::Error> {
    let row = sqlx::query_as!(
        RankSnapshot,
        r#"
        SELECT badge_level AS "badge_level!",
               rank_name AS "rank_name!",
               captured_at AS "captured_at!"
        FROM steam.steam_rank_history
        WHERE user_id = $1
          AND badge_level IS NOT NULL
          AND rank_name IS NOT NULL
        ORDER BY captured_at DESC, badge_level DESC
        LIMIT 1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Laedt Rank-History fuer einen User. `days = 0` bedeutet All-Time; die
/// Baseline ist dann der aelteste verfuegbare Eintrag.
async fn rank_history(
    pool: &PgPool,
    user_id: i64,
    days: i64,
    now: DateTime<Utc>,
) -> Result<Vec<RankSnapshot>, sqlx::Error> {
    let cutoff = if days == 0 {
        None
    } else {
        Some(now - Duration::days(days))
    };
    let rows = sqlx::query_as!(
        RankSnapshot,
        r#"
        SELECT badge_level AS "badge_level!",
               rank_name AS "rank_name!",
               captured_at AS "captured_at!"
        FROM steam.steam_rank_history
        WHERE user_id = $1
          AND ($2::timestamptz IS NULL OR captured_at >= $2)
          AND badge_level IS NOT NULL
          AND rank_name IS NOT NULL
        ORDER BY captured_at ASC, badge_level DESC
        "#,
        user_id,
        cutoff,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn visibility_of(pool: &PgPool, user_id: i64) -> Result<RankVisibility, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT visibility AS "visibility?"
        FROM steam.rank_history_visibility
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(visibility_from_db(row.and_then(|row| row.visibility)))
}

async fn set_visibility(
    pool: &PgPool,
    user_id: i64,
    visibility: RankVisibility,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO steam.rank_history_visibility(user_id, visibility, updated_at)
        VALUES($1, $2, now())
        ON CONFLICT (user_id) DO UPDATE SET
            visibility = EXCLUDED.visibility,
            updated_at = now()
        "#,
        user_id,
        visibility.as_str(),
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn viewer_is_guild_member(
    pool: &PgPool,
    viewer_id: Option<i64>,
) -> Result<bool, sqlx::Error> {
    let Some(user_id) = viewer_id else {
        return Ok(false);
    };
    let row = sqlx::query!(
        r#"
        SELECT event_type AS "event_type!"
        FROM activity.member_events
        WHERE user_id = $1
          AND guild_id = $2
        ORDER BY occurred_at DESC NULLS LAST, id DESC
        LIMIT 1
        "#,
        user_id,
        MAIN_GUILD_ID,
    )
    .fetch_optional(pool)
    .await?;
    Ok(matches!(
        row.as_ref().map(|row| row.event_type.as_str()),
        Some("join")
    ))
}

async fn me_rank_history_payload(
    pool: &PgPool,
    user_id: i64,
    days: i64,
    now: DateTime<Utc>,
) -> Result<MeRankHistoryPayload, sqlx::Error> {
    let current = current_rank(pool, user_id).await?;
    let history = rank_history(pool, user_id, days, now).await?;
    let visibility = visibility_of(pool, user_id).await?;
    Ok(MeRankHistoryPayload {
        current: current.map(RankEntry::from),
        history: history.into_iter().map(RankEntry::from).collect(),
        visibility: visibility.as_str().to_string(),
    })
}

async fn public_rank_history_payload_for_viewer(
    pool: &PgPool,
    viewer_id: Option<i64>,
    target_user_id: i64,
    days: i64,
    now: DateTime<Utc>,
) -> Result<Option<PublicRankHistoryPayload>, sqlx::Error> {
    let viewer_is_member = viewer_is_guild_member(pool, viewer_id).await?;
    let visibility = visibility_of(pool, target_user_id).await?;
    if !visible_to_viewer(visibility, viewer_is_member) {
        return Ok(None);
    }

    let Some(current) = current_rank(pool, target_user_id).await? else {
        return Ok(None);
    };
    let history = rank_history(pool, target_user_id, days, now).await?;
    Ok(Some(PublicRankHistoryPayload {
        current: RankEntry::from(current),
        history: history.into_iter().map(RankEntry::from).collect(),
    }))
}

async fn visible_leaderboard_rows(
    pool: &PgPool,
    viewer_is_member: bool,
    sort: LeaderboardSort,
    days: i64,
    limit: i64,
    now: DateTime<Utc>,
) -> Result<Vec<LeaderboardRow>, sqlx::Error> {
    let allowed = allowed_visibility_values(viewer_is_member);
    let cutoff = if days == 0 {
        None
    } else {
        Some(now - Duration::days(days))
    };
    let rows = sqlx::query!(
        r#"
        WITH visible_users AS (
            SELECT visibility.user_id
            FROM steam.rank_history_visibility visibility
            WHERE visibility.visibility = ANY($1)
        ),
        ranked AS (
            SELECT visible.user_id,
                   current_rank.badge_level,
                   current_rank.rank_name,
                   current_rank.captured_at,
                   current_rank.badge_level
                       - COALESCE(baseline_before.badge_level, baseline_oldest.badge_level) AS delta
            FROM visible_users visible
            JOIN LATERAL (
                SELECT history.badge_level,
                       history.rank_name,
                       history.captured_at
                FROM steam.steam_rank_history history
                WHERE history.user_id = visible.user_id
                  AND history.badge_level IS NOT NULL
                  AND history.rank_name IS NOT NULL
                ORDER BY history.captured_at DESC, history.badge_level DESC
                LIMIT 1
            ) current_rank ON TRUE
            JOIN LATERAL (
                SELECT history.badge_level
                FROM steam.steam_rank_history history
                WHERE history.user_id = visible.user_id
                  AND history.badge_level IS NOT NULL
                  AND history.rank_name IS NOT NULL
                ORDER BY history.captured_at ASC, history.badge_level DESC
                LIMIT 1
            ) baseline_oldest ON TRUE
            LEFT JOIN LATERAL (
                SELECT history.badge_level
                FROM steam.steam_rank_history history
                WHERE $2::timestamptz IS NOT NULL
                  AND history.user_id = visible.user_id
                  AND history.captured_at <= $2
                  AND history.badge_level IS NOT NULL
                  AND history.rank_name IS NOT NULL
                ORDER BY history.captured_at DESC, history.badge_level DESC
                LIMIT 1
            ) baseline_before ON TRUE
        )
        SELECT ranked.user_id AS "user_id!",
               ranked.badge_level AS "badge_level!",
               ranked.rank_name AS "rank_name!",
               ranked.captured_at AS "captured_at!",
               ranked.delta AS "delta!"
        FROM ranked
        ORDER BY
            CASE WHEN $3::text = 'climb' THEN ranked.delta END DESC NULLS LAST,
            CASE WHEN $3::text = 'climb' THEN ranked.badge_level END DESC NULLS LAST,
            CASE WHEN $3::text = 'top' THEN ranked.badge_level END DESC NULLS LAST,
            CASE WHEN $3::text = 'top' THEN ranked.delta END DESC NULLS LAST,
            ranked.user_id ASC
        LIMIT $4
        "#,
        &allowed[..],
        cutoff,
        sort.as_str(),
        limit,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| LeaderboardRow {
            user_id: row.user_id,
            badge_level: row.badge_level,
            rank_name: row.rank_name,
            delta: row.delta,
        })
        .collect())
}

async fn rank_leaderboard_payload_for_viewer(
    pool: &PgPool,
    viewer_id: Option<i64>,
    sort: LeaderboardSort,
    days: i64,
    limit: i64,
    now: DateTime<Utc>,
) -> Result<Vec<LeaderboardEntry>, sqlx::Error> {
    let viewer_is_member = viewer_is_guild_member(pool, viewer_id).await?;
    let rows = visible_leaderboard_rows(pool, viewer_is_member, sort, days, limit, now).await?;

    let ids: Vec<i64> = rows.iter().map(|row| row.user_id).collect();
    let names = resolve_display_names(pool, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| LeaderboardEntry {
            user_id: row.user_id.to_string(),
            display_name: names
                .get(&row.user_id)
                .cloned()
                .unwrap_or_else(|| format!("User {}", row.user_id)),
            rank_name: row.rank_name,
            badge_level: row.badge_level,
            delta: row.delta,
        })
        .collect())
}

/// `GET /api/public/me/rank-history`.
///
/// Query `days=0` bedeutet All-Time; hoehere Werte werden auf
/// `MAX_RANK_HISTORY_DAYS` geklemmt, negative/nicht-numerische Werte sind 400.
pub async fn handle_me_rank_history(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    let days = match parse_history_days(params.get("days")) {
        Ok(days) => days,
        Err(resp) => return resp,
    };

    match me_rank_history_payload(&app.pool, user_id, days, Utc::now()).await {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

/// `PUT /api/public/me/rank-visibility`.
pub async fn handle_me_rank_visibility(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    body: Result<Json<VisibilityRequest>, JsonRejection>,
) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    let Json(payload) = match body {
        Ok(body) => body,
        Err(_) => return invalid_visibility_response(),
    };
    let visibility = match parse_visibility(&payload.visibility) {
        Some(visibility) => visibility,
        None => return invalid_visibility_response(),
    };

    match set_visibility(&app.pool, user_id, visibility).await {
        Ok(()) => Json(json!({ "visibility": visibility.as_str() })).into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/public/leaderboard/rank`.
///
/// Query `days=0` bedeutet All-Time; hoehere Werte werden auf
/// `MAX_RANK_HISTORY_DAYS` geklemmt, negative/nicht-numerische Werte sind 400.
pub async fn handle_rank_leaderboard(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let sort = match parse_sort(params.get("sort")) {
        Ok(sort) => sort,
        Err(resp) => return resp,
    };
    let days = match parse_leaderboard_days(params.get("days")) {
        Ok(days) => days,
        Err(resp) => return resp,
    };
    let limit = match parse_positive_int(params.get("limit"), 50, 1, 100, "limit") {
        Ok(limit) => limit,
        Err(resp) => return resp,
    };
    let viewer_id = optional_session_user_id(&app, &headers);

    match rank_leaderboard_payload_for_viewer(&app.pool, viewer_id, sort, days, limit, Utc::now())
        .await
    {
        Ok(entries) => Json(json!({ "entries": entries })).into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/public/rank-history/{user_id}`.
///
/// Query `days=0` bedeutet All-Time; hoehere Werte werden auf
/// `MAX_RANK_HISTORY_DAYS` geklemmt, negative/nicht-numerische Werte sind 400.
pub async fn handle_public_rank_history(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Path(raw_user_id): Path<String>,
    Query(params): Params,
) -> Response {
    let Some(target_user_id) = parse_target_user_id(&raw_user_id) else {
        return not_found_response();
    };
    let days = match parse_history_days(params.get("days")) {
        Ok(days) => days,
        Err(resp) => return resp,
    };
    let viewer_id = optional_session_user_id(&app, &headers);

    match public_rank_history_payload_for_viewer(
        &app.pool,
        viewer_id,
        target_user_id,
        days,
        Utc::now(),
    )
    .await
    {
        Ok(Some(payload)) => Json(payload).into_response(),
        Ok(None) => not_found_response(),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    #[cfg(feature = "testing")]
    use std::collections::HashSet;

    fn ts(raw: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
        Ok(DateTime::parse_from_rfc3339(raw)?.with_timezone(&Utc))
    }

    #[test]
    fn visibility_validation_akzeptiert_nur_vertragswerte() {
        assert_eq!(parse_visibility("private"), Some(RankVisibility::Private));
        assert_eq!(parse_visibility("members"), Some(RankVisibility::Members));
        assert_eq!(parse_visibility("public"), Some(RankVisibility::Public));
        assert_eq!(parse_visibility("PUBLIC"), None);
        assert_eq!(parse_visibility("friends"), None);
        assert_eq!(parse_visibility(""), None);
    }

    #[test]
    fn visibility_filter_members_nur_fuer_mitglieder() {
        assert!(visible_to_viewer(RankVisibility::Public, false));
        assert!(visible_to_viewer(RankVisibility::Public, true));
        assert!(!visible_to_viewer(RankVisibility::Members, false));
        assert!(visible_to_viewer(RankVisibility::Members, true));
        assert!(!visible_to_viewer(RankVisibility::Private, true));
    }

    #[test]
    fn delta_nimmt_jungsten_eintrag_mindestens_days_alt() -> Result<(), chrono::ParseError> {
        let now = ts("2026-07-03T00:00:00Z")?;
        let history = vec![
            RankSnapshot {
                badge_level: 20,
                rank_name: "Seeker".to_string(),
                captured_at: ts("2026-05-01T00:00:00Z")?,
            },
            RankSnapshot {
                badge_level: 25,
                rank_name: "Alchemist".to_string(),
                captured_at: ts("2026-06-01T00:00:00Z")?,
            },
            RankSnapshot {
                badge_level: 30,
                rank_name: "Arcanist".to_string(),
                captured_at: ts("2026-07-01T00:00:00Z")?,
            },
        ];

        assert_eq!(rank_delta(&history, 30, now), Some(5));
        assert_eq!(rank_delta(&history, 90, now), Some(10));
        Ok(())
    }

    #[test]
    fn days_null_bedeutet_all_time_mit_aeltester_baseline() -> Result<(), chrono::ParseError> {
        let now = ts("2026-07-03T00:00:00Z")?;
        let history = vec![
            RankSnapshot {
                badge_level: 10,
                rank_name: "Seeker".to_string(),
                captured_at: ts("2025-01-01T00:00:00Z")?,
            },
            RankSnapshot {
                badge_level: 20,
                rank_name: "Alchemist".to_string(),
                captured_at: ts("2026-06-01T00:00:00Z")?,
            },
            RankSnapshot {
                badge_level: 30,
                rank_name: "Arcanist".to_string(),
                captured_at: ts("2026-07-01T00:00:00Z")?,
            },
        ];

        assert_eq!(rank_delta(&history, 0, now), Some(20));
        assert_eq!(rank_delta(&history, 30, now), Some(10));
        Ok(())
    }

    #[test]
    fn days_parser_klemmt_obergrenze_und_lehnt_negative_werte_ab() {
        let zero = "0".to_string();
        let large = "99999".to_string();
        let negative = "-1".to_string();
        let invalid = "abc".to_string();

        assert_eq!(parse_history_days(Some(&zero)).expect("days=0"), 0);
        assert_eq!(
            parse_history_days(Some(&large)).expect("large days is clamped"),
            MAX_RANK_HISTORY_DAYS
        );
        assert_eq!(
            parse_history_days(Some(&negative))
                .expect_err("negative days")
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_history_days(Some(&invalid))
                .expect_err("invalid days")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn private_und_unknown_not_found_body_bleibt_gleich() -> Result<(), axum::Error> {
        let private = not_found_response();
        let unknown = not_found_response();
        assert_eq!(private.status(), StatusCode::NOT_FOUND);
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

        let private_body = to_bytes(private.into_body(), 1024).await?;
        let unknown_body = to_bytes(unknown.into_body(), 1024).await?;
        assert_eq!(private_body, unknown_body);
        Ok(())
    }

    #[cfg(feature = "testing")]
    async fn insert_rank(
        pool: &PgPool,
        user_id: i64,
        badge_level: i32,
        rank_name: &str,
        captured_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO steam.steam_rank_history(user_id, badge_level, rank_name, captured_at)
            VALUES($1, $2, $3, $4)
            "#,
        )
        .bind(user_id)
        .bind(badge_level)
        .bind(rank_name)
        .bind(captured_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    #[cfg(feature = "testing")]
    async fn insert_visibility(
        pool: &PgPool,
        user_id: i64,
        visibility: RankVisibility,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO steam.rank_history_visibility(user_id, visibility)
            VALUES($1, $2)
            "#,
        )
        .bind(user_id)
        .bind(visibility.as_str())
        .execute(pool)
        .await?;
        Ok(())
    }

    #[cfg(feature = "testing")]
    async fn insert_member_event(
        pool: &PgPool,
        id: i64,
        user_id: i64,
        event_type: &str,
        occurred_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO activity.member_events(
                id, user_id, guild_id, event_type, occurred_at, display_name
            )
            VALUES($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(MAIN_GUILD_ID)
        .bind(event_type)
        .bind(occurred_at)
        .bind(format!("User {user_id}"))
        .execute(pool)
        .await?;
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn leaderboard_filtert_public_members_private_nach_betrachter(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let now = ts("2026-07-03T00:00:00Z")?;

        insert_rank(pool, 1, 20, "Seeker", ts("2026-06-01T00:00:00Z")?).await?;
        insert_rank(pool, 1, 25, "Alchemist", ts("2026-07-01T00:00:00Z")?).await?;
        insert_rank(pool, 2, 30, "Arcanist", ts("2026-06-01T00:00:00Z")?).await?;
        insert_rank(pool, 2, 28, "Arcanist", ts("2026-07-01T00:00:00Z")?).await?;
        insert_rank(pool, 3, 40, "Oracle", ts("2026-07-01T00:00:00Z")?).await?;

        insert_visibility(pool, 1, RankVisibility::Public).await?;
        insert_visibility(pool, 2, RankVisibility::Members).await?;
        insert_visibility(pool, 3, RankVisibility::Private).await?;
        insert_member_event(pool, 10_001, 100, "join", ts("2026-06-01T00:00:00Z")?).await?;
        insert_member_event(pool, 10_002, 101, "join", ts("2026-06-01T00:00:00Z")?).await?;
        insert_member_event(pool, 10_003, 101, "leave", ts("2026-07-01T00:00:00Z")?).await?;

        let anon =
            rank_leaderboard_payload_for_viewer(pool, None, LeaderboardSort::Top, 30, 50, now)
                .await?;
        assert_eq!(
            anon.iter()
                .map(|entry| entry.user_id.as_str())
                .collect::<Vec<_>>(),
            vec!["1"]
        );

        let member =
            rank_leaderboard_payload_for_viewer(pool, Some(100), LeaderboardSort::Top, 30, 50, now)
                .await?;
        assert_eq!(
            member
                .iter()
                .map(|entry| entry.user_id.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["1", "2"])
        );
        let member_delta = member
            .iter()
            .find(|entry| entry.user_id == "2")
            .map(|entry| entry.delta);
        assert_eq!(member_delta, Some(-2));

        let former_member =
            rank_leaderboard_payload_for_viewer(pool, Some(101), LeaderboardSort::Top, 30, 50, now)
                .await?;
        assert_eq!(
            former_member
                .iter()
                .map(|entry| entry.user_id.as_str())
                .collect::<Vec<_>>(),
            vec!["1"]
        );
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn mitgliedschaft_nur_wenn_neuestes_event_join_ist(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        insert_member_event(pool, 20_001, 200, "join", ts("2026-06-01T00:00:00Z")?).await?;
        insert_member_event(pool, 20_002, 200, "ban", ts("2026-06-02T00:00:00Z")?).await?;
        insert_member_event(pool, 20_003, 200, "unban", ts("2026-06-03T00:00:00Z")?).await?;

        insert_member_event(pool, 20_004, 201, "join", ts("2026-06-01T00:00:00Z")?).await?;
        insert_member_event(pool, 20_005, 201, "leave", ts("2026-06-02T00:00:00Z")?).await?;
        insert_member_event(pool, 20_006, 201, "join", ts("2026-06-03T00:00:00Z")?).await?;

        assert!(!viewer_is_guild_member(pool, Some(200)).await?);
        assert!(viewer_is_guild_member(pool, Some(201)).await?);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn leaderboard_days_null_nutzt_all_time_baseline(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let now = ts("2026-07-03T00:00:00Z")?;

        insert_rank(pool, 4, 5, "Seeker", ts("2025-01-01T00:00:00Z")?).await?;
        insert_rank(pool, 4, 20, "Alchemist", ts("2026-06-01T00:00:00Z")?).await?;
        insert_rank(pool, 4, 30, "Arcanist", ts("2026-07-01T00:00:00Z")?).await?;
        insert_visibility(pool, 4, RankVisibility::Public).await?;

        let all_time =
            rank_leaderboard_payload_for_viewer(pool, None, LeaderboardSort::Climb, 0, 50, now)
                .await?;
        let thirty_days =
            rank_leaderboard_payload_for_viewer(pool, None, LeaderboardSort::Climb, 30, 50, now)
                .await?;

        assert_eq!(all_time[0].delta, 25);
        assert_eq!(thirty_days[0].delta, 10);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn private_und_unknown_liefern_gleiche_payload_absenz(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let now = ts("2026-07-03T00:00:00Z")?;
        insert_rank(pool, 9, 25, "Alchemist", ts("2026-07-01T00:00:00Z")?).await?;
        insert_visibility(pool, 9, RankVisibility::Private).await?;

        let private = public_rank_history_payload_for_viewer(pool, None, 9, 30, now).await?;
        let unknown = public_rank_history_payload_for_viewer(pool, None, 404, 30, now).await?;
        assert!(private.is_none());
        assert!(unknown.is_none());
        Ok(())
    }
}
