use std::collections::HashSet;
use std::num::TryFromIntError;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use tokio::time::Instant;

pub const GC_CREATE_CUSTOM_LOBBY: &str = "GC_CREATE_CUSTOM_LOBBY";
pub const GC_LOBBY_INVITE_PLAYER: &str = "GC_LOBBY_INVITE_PLAYER";
pub const GC_LOBBY_SET_SPECTATOR: &str = "GC_LOBBY_SET_SPECTATOR";
pub const GC_LOBBY_APPLY_CONVARS: &str = "GC_LOBBY_APPLY_CONVARS";
pub const GC_LOBBY_READY: &str = "GC_LOBBY_READY";
pub const GC_LOBBY_START_MATCH: &str = "GC_LOBBY_START_MATCH";
pub const GC_LOBBY_LEAVE: &str = "GC_LOBBY_LEAVE";
pub const GC_GET_MATCH_RESULT: &str = "GC_GET_MATCH_RESULT";

const STATE_LOBBY_CREATED: &str = "lobby_created";
const STATE_INVITED: &str = "invited";
const STATE_SPECTATOR: &str = "spectator";
const STATE_CONVARS_APPLIED: &str = "convars_applied";
const STATE_READY: &str = "ready";
const STATE_IN_PROGRESS: &str = "in_progress";
const STATE_FINISHED: &str = "finished";

const CREATE_LOBBY_TIMEOUT: Duration = Duration::from_secs(45);
const INVITE_TIMEOUT: Duration = Duration::from_secs(30);
const SET_SPECTATOR_TIMEOUT: Duration = Duration::from_secs(20);
const APPLY_CONVARS_TIMEOUT: Duration = Duration::from_secs(30);
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const START_MATCH_TIMEOUT: Duration = Duration::from_secs(45);
const RESULT_TIMEOUT: Duration = Duration::from_secs(45);
const LEAVE_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum ScrimMatchErr {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("Scrim-Match fehlt: id {0}")]
    MissingMatch(i64),
    #[error("Scrim-Match {match_id} fehlt {field}")]
    MissingMatchTeam { match_id: i64, field: &'static str },
    #[error("Scrim-ID {label}={value} passt nicht in PostgreSQL int4")]
    IdOutOfRange {
        label: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("Steam task {0} fehlt")]
    SteamTaskMissing(i64),
    #[error("Steam task {task_id} fehlgeschlagen: {error}")]
    SteamTaskFailed { task_id: i64, error: String },
    #[error("Steam task {task_id} Timeout nach {timeout_secs}s")]
    SteamTaskTimeout { task_id: i64, timeout_secs: u64 },
    #[error("{field} fehlt im Steam-Ergebnis")]
    MissingTaskResultField { field: &'static str },
    #[error("{field} muss eine ganze Zahl sein")]
    InvalidTaskResultInt { field: &'static str },
    #[error("winning_team={winning_team} passt nicht zu Team A/B")]
    InvalidWinningTeam { winning_team: i64 },
    #[error("GC_GET_MATCH_RESULT braucht steam_match_id oder party_id")]
    MissingResultLookup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartScrimMatchOutcome {
    pub party_id: String,
    pub join_code: String,
    pub steam_match_id: Option<i64>,
    pub invited_participants: Vec<(i64, i64)>,
    pub unlinked_participants: Vec<i64>,
    pub coach_spectator_steam_id64: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrimMatchResultOutcome {
    pub steam_match_id: Option<i64>,
    pub winner_team_id: Option<i64>,
    pub result_json: Value,
}

#[derive(Debug, Clone)]
struct ScrimMatchRow {
    team_a_id: Option<i64>,
    team_b_id: Option<i64>,
    party_id: Option<String>,
    steam_match_id: Option<i64>,
    coach_spectator_discord_id: Option<i64>,
}

pub async fn resolve_team_steam_ids(
    pool: &PgPool,
    team_id: i64,
) -> Result<(Vec<(i64, i64)>, Vec<i64>), ScrimMatchErr> {
    let team_id = to_i32_id("team_id", team_id)?;
    let rows = sqlx::query(
        r#"
        SELECT tm.participant_id::bigint AS participant_id,
               linked.steam_id64
          FROM scrim.team_members tm
          JOIN scrim.participants p ON p.id = tm.participant_id
          LEFT JOIN LATERAL (
              SELECT sl.steam_id64
                FROM core.steam_links sl
               WHERE sl.discord_id = p.discord_id
                 AND sl.steam_id64 IS NOT NULL
               ORDER BY sl.primary_account DESC,
                        sl.verified DESC,
                        sl.updated_at DESC NULLS LAST,
                        sl.steam_id ASC
               LIMIT 1
          ) linked ON TRUE
         WHERE tm.team_id = $1
         ORDER BY tm.participant_id ASC
        "#,
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?;

    let mut linked = Vec::new();
    let mut unlinked = Vec::new();
    for row in rows {
        let participant_id: i64 = row.get("participant_id");
        if let Some(steam_id64) = row.get::<Option<i64>, _>("steam_id64") {
            linked.push((participant_id, steam_id64));
        } else {
            unlinked.push(participant_id);
        }
    }
    Ok((linked, unlinked))
}

pub async fn start_scrim_match(
    pool: &PgPool,
    match_id: i64,
) -> Result<StartScrimMatchOutcome, ScrimMatchErr> {
    let scrim_match = load_scrim_match(pool, match_id).await?;
    let team_a_id = required_team_id(&scrim_match, match_id, "team_a_id")?;
    let team_b_id = required_team_id(&scrim_match, match_id, "team_b_id")?;
    let (team_a_linked, mut unlinked_participants) =
        resolve_team_steam_ids(pool, team_a_id).await?;
    let (team_b_linked, team_b_unlinked) = resolve_team_steam_ids(pool, team_b_id).await?;
    unlinked_participants.extend(team_b_unlinked);

    let create_result = run_steam_task(
        pool,
        GC_CREATE_CUSTOM_LOBBY,
        build_create_custom_lobby_payload(),
        CREATE_LOBBY_TIMEOUT,
    )
    .await?;
    let party_id = required_string(create_result.get("party_id"), "party_id")?;
    let join_code = resolve_join_code(&create_result)?;
    update_lobby_created(pool, match_id, &party_id, &join_code).await?;

    let mut invited_participants = Vec::new();
    let mut invited_steam_ids = HashSet::new();
    for (participant_id, steam_id64) in team_a_linked.iter().chain(team_b_linked.iter()).copied() {
        if invited_steam_ids.insert(steam_id64) {
            invite_steam_id64(pool, &party_id, steam_id64).await?;
        }
        invited_participants.push((participant_id, steam_id64));
    }
    update_lobby_state(pool, match_id, STATE_INVITED).await?;

    let coach_spectator_steam_id64 =
        if let Some(discord_id) = scrim_match.coach_spectator_discord_id {
            let steam_id64 = resolve_discord_steam_id64(pool, discord_id).await?;
            if let Some(steam_id64) = steam_id64 {
                if invited_steam_ids.insert(steam_id64) {
                    invite_steam_id64(pool, &party_id, steam_id64).await?;
                }
            }
            steam_id64
        } else {
            None
        };

    run_steam_task(
        pool,
        GC_LOBBY_SET_SPECTATOR,
        build_set_spectator_payload(&party_id),
        SET_SPECTATOR_TIMEOUT,
    )
    .await?;
    update_lobby_state(pool, match_id, STATE_SPECTATOR).await?;

    run_steam_task(
        pool,
        GC_LOBBY_APPLY_CONVARS,
        build_apply_convars_payload(&party_id),
        APPLY_CONVARS_TIMEOUT,
    )
    .await?;
    update_lobby_state(pool, match_id, STATE_CONVARS_APPLIED).await?;

    run_steam_task(
        pool,
        GC_LOBBY_READY,
        build_ready_payload(&party_id),
        READY_TIMEOUT,
    )
    .await?;
    update_lobby_state(pool, match_id, STATE_READY).await?;

    let start_result = run_steam_task(
        pool,
        GC_LOBBY_START_MATCH,
        build_start_match_payload(&party_id),
        START_MATCH_TIMEOUT,
    )
    .await?;
    let steam_match_id = first_match_id(&start_result)?;
    update_match_started(pool, match_id, steam_match_id).await?;

    Ok(StartScrimMatchOutcome {
        party_id,
        join_code,
        steam_match_id,
        invited_participants,
        unlinked_participants,
        coach_spectator_steam_id64,
    })
}

pub async fn fetch_scrim_match_result(
    pool: &PgPool,
    match_id: i64,
) -> Result<ScrimMatchResultOutcome, ScrimMatchErr> {
    fetch_scrim_match_result_with_lookup(pool, match_id, None).await
}

pub async fn fetch_scrim_match_result_by_steam_match_id(
    pool: &PgPool,
    match_id: i64,
    steam_match_id: i64,
) -> Result<ScrimMatchResultOutcome, ScrimMatchErr> {
    fetch_scrim_match_result_with_lookup(pool, match_id, Some(steam_match_id)).await
}

async fn fetch_scrim_match_result_with_lookup(
    pool: &PgPool,
    match_id: i64,
    lookup_steam_match_id: Option<i64>,
) -> Result<ScrimMatchResultOutcome, ScrimMatchErr> {
    let scrim_match = load_scrim_match(pool, match_id).await?;
    let team_a_id = required_team_id(&scrim_match, match_id, "team_a_id")?;
    let team_b_id = required_team_id(&scrim_match, match_id, "team_b_id")?;
    let payload = build_match_result_payload(
        lookup_steam_match_id.or(scrim_match.steam_match_id),
        scrim_match.party_id.as_deref(),
    );
    if payload.as_object().is_none_or(serde_json::Map::is_empty) {
        return Err(ScrimMatchErr::MissingResultLookup);
    }

    let result = run_steam_task(pool, GC_GET_MATCH_RESULT, payload, RESULT_TIMEOUT).await?;
    let steam_match_id = first_match_id(&result)?.or(scrim_match.steam_match_id);
    let winner_team_id = winner_team_id(result.get("winning_team"), team_a_id, team_b_id)?;

    if let Some(party_id) = scrim_match
        .party_id
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    {
        run_steam_task(
            pool,
            GC_LOBBY_LEAVE,
            build_leave_payload(party_id),
            LEAVE_TIMEOUT,
        )
        .await?;
    }

    update_match_finished(pool, match_id, steam_match_id, winner_team_id, &result).await?;
    Ok(ScrimMatchResultOutcome {
        steam_match_id,
        winner_team_id,
        result_json: result,
    })
}

pub fn build_create_custom_lobby_payload() -> Value {
    json!({
        "game_mode": 1,
        "region_mode": 1,
        "convars": {},
        "is_private": true,
    })
}

pub fn build_invite_player_payload(party_id: &str, steam_id64: i64) -> Value {
    json!({
        "steam_id": steam_id64.to_string(),
        "party_id": party_id,
    })
}

pub fn build_party_id_payload(party_id: &str) -> Value {
    json!({ "party_id": party_id })
}

pub fn build_set_spectator_payload(party_id: &str) -> Value {
    build_party_id_payload(party_id)
}

pub fn build_ready_payload(party_id: &str) -> Value {
    build_party_id_payload(party_id)
}

pub fn build_start_match_payload(party_id: &str) -> Value {
    build_party_id_payload(party_id)
}

pub fn build_leave_payload(party_id: &str) -> Value {
    build_party_id_payload(party_id)
}

pub fn build_apply_convars_payload(party_id: &str) -> Value {
    json!({
        "party_id": party_id,
        "convars": default_scrim_convars(),
    })
}

pub fn build_match_result_payload(steam_match_id: Option<i64>, party_id: Option<&str>) -> Value {
    let mut payload = serde_json::Map::new();
    if let Some(steam_match_id) = steam_match_id {
        payload.insert("match_id".to_string(), json!(steam_match_id));
    }
    if let Some(party_id) = party_id
        .map(str::trim)
        .filter(|party_id| !party_id.is_empty())
    {
        payload.insert("party_id".to_string(), json!(party_id));
    }
    Value::Object(payload)
}

fn default_scrim_convars() -> Value {
    json!({ "citadel_allow_duplicate_heroes": 0 })
}

async fn invite_steam_id64(
    pool: &PgPool,
    party_id: &str,
    steam_id64: i64,
) -> Result<(), ScrimMatchErr> {
    run_steam_task(
        pool,
        GC_LOBBY_INVITE_PLAYER,
        build_invite_player_payload(party_id, steam_id64),
        INVITE_TIMEOUT,
    )
    .await?;
    Ok(())
}

async fn run_steam_task(
    pool: &PgPool,
    task_type: &str,
    payload: Value,
    timeout: Duration,
) -> Result<Value, ScrimMatchErr> {
    let task_id = enqueue_steam_task(pool, task_type, payload).await?;
    poll_steam_task(pool, task_id, timeout).await
}

async fn enqueue_steam_task(
    pool: &PgPool,
    task_type: &str,
    payload: Value,
) -> Result<i64, ScrimMatchErr> {
    let task_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO steam.steam_tasks(type, payload, status)
        VALUES($1, $2, 'PENDING')
        RETURNING id
        "#,
    )
    .bind(task_type)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    Ok(task_id)
}

async fn poll_steam_task(
    pool: &PgPool,
    task_id: i64,
    timeout: Duration,
) -> Result<Value, ScrimMatchErr> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(ScrimMatchErr::SteamTaskTimeout {
                task_id,
                timeout_secs: timeout.as_secs(),
            });
        }

        let row = sqlx::query(
            r#"
            SELECT status,
                   result,
                   error
              FROM steam.steam_tasks
             WHERE id = $1
            "#,
        )
        .bind(task_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ScrimMatchErr::SteamTaskMissing(task_id))?;

        let status: String = row.get("status");
        match status.as_str() {
            "DONE" => {
                let result = row
                    .get::<Option<Value>, _>("result")
                    .unwrap_or_else(|| json!({ "success": true }));
                return Ok(result);
            }
            "FAILED" => {
                let error = row
                    .get::<Option<String>, _>("error")
                    .filter(|error| !error.is_empty())
                    .unwrap_or_else(|| "Steam-Task fehlgeschlagen".to_string());
                return Err(ScrimMatchErr::SteamTaskFailed { task_id, error });
            }
            _ => tokio::time::sleep(POLL_INTERVAL).await,
        }
    }
}

async fn load_scrim_match(pool: &PgPool, match_id: i64) -> Result<ScrimMatchRow, ScrimMatchErr> {
    let match_id_i32 = to_i32_id("match_id", match_id)?;
    let row = sqlx::query(
        r#"
        SELECT team_a_id::bigint AS team_a_id,
               team_b_id::bigint AS team_b_id,
               party_id,
               steam_match_id,
               coach_spectator_discord_id
          FROM scrim.matches
         WHERE id = $1
        "#,
    )
    .bind(match_id_i32)
    .fetch_optional(pool)
    .await?
    .ok_or(ScrimMatchErr::MissingMatch(match_id))?;

    Ok(ScrimMatchRow {
        team_a_id: row.get("team_a_id"),
        team_b_id: row.get("team_b_id"),
        party_id: row.get("party_id"),
        steam_match_id: row.get("steam_match_id"),
        coach_spectator_discord_id: row.get("coach_spectator_discord_id"),
    })
}

async fn resolve_discord_steam_id64(
    pool: &PgPool,
    discord_id: i64,
) -> Result<Option<i64>, ScrimMatchErr> {
    let steam_id64 = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT steam_id64
          FROM core.steam_links
         WHERE discord_id = $1
           AND steam_id64 IS NOT NULL
         ORDER BY primary_account DESC,
                  verified DESC,
                  updated_at DESC NULLS LAST,
                  steam_id ASC
         LIMIT 1
        "#,
    )
    .bind(discord_id)
    .fetch_optional(pool)
    .await?;
    Ok(steam_id64)
}

async fn update_lobby_created(
    pool: &PgPool,
    match_id: i64,
    party_id: &str,
    join_code: &str,
) -> Result<(), ScrimMatchErr> {
    let match_id = to_i32_id("match_id", match_id)?;
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET party_id = $2,
               join_code = $3,
               lobby_state = $4,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(match_id)
    .bind(party_id)
    .bind(join_code)
    .bind(STATE_LOBBY_CREATED)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_lobby_state(
    pool: &PgPool,
    match_id: i64,
    lobby_state: &str,
) -> Result<(), ScrimMatchErr> {
    let match_id = to_i32_id("match_id", match_id)?;
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET lobby_state = $2,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(match_id)
    .bind(lobby_state)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_match_started(
    pool: &PgPool,
    match_id: i64,
    steam_match_id: Option<i64>,
) -> Result<(), ScrimMatchErr> {
    let match_id = to_i32_id("match_id", match_id)?;
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET steam_match_id = COALESCE($2, steam_match_id),
               lobby_state = $3,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(match_id)
    .bind(steam_match_id)
    .bind(STATE_IN_PROGRESS)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_match_finished(
    pool: &PgPool,
    match_id: i64,
    steam_match_id: Option<i64>,
    winner_team_id: Option<i64>,
    result_json: &Value,
) -> Result<(), ScrimMatchErr> {
    let match_id = to_i32_id("match_id", match_id)?;
    let winner_team_id = winner_team_id
        .map(|id| to_i32_id("winner_team_id", id))
        .transpose()?;
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET steam_match_id = COALESCE($2, steam_match_id),
               winner_team_id = $3,
               result_json = $4,
               lobby_state = $5,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(match_id)
    .bind(steam_match_id)
    .bind(winner_team_id)
    .bind(result_json)
    .bind(STATE_FINISHED)
    .execute(pool)
    .await?;
    Ok(())
}

fn required_team_id(
    scrim_match: &ScrimMatchRow,
    match_id: i64,
    field: &'static str,
) -> Result<i64, ScrimMatchErr> {
    match field {
        "team_a_id" => scrim_match.team_a_id,
        "team_b_id" => scrim_match.team_b_id,
        _ => None,
    }
    .ok_or(ScrimMatchErr::MissingMatchTeam { match_id, field })
}

fn required_string(value: Option<&Value>, field: &'static str) -> Result<String, ScrimMatchErr> {
    let out = match value {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    };
    if out.is_empty() {
        return Err(ScrimMatchErr::MissingTaskResultField { field });
    }
    Ok(out)
}

fn resolve_join_code(result: &Value) -> Result<String, ScrimMatchErr> {
    for key in ["join_code", "party_code", "party_code_display"] {
        if let Some(value) = result.get(key) {
            if let Ok(code) = required_string(Some(value), "join_code") {
                return Ok(code);
            }
        }
    }
    Err(ScrimMatchErr::MissingTaskResultField { field: "join_code" })
}

fn first_match_id(result: &Value) -> Result<Option<i64>, ScrimMatchErr> {
    for key in ["match_id", "deadlock_match_id"] {
        match result.get(key) {
            None | Some(Value::Null) => {}
            Some(value) => return optional_i64(Some(value), key),
        }
    }
    Ok(None)
}

fn winner_team_id(
    winning_team: Option<&Value>,
    team_a_id: i64,
    team_b_id: i64,
) -> Result<Option<i64>, ScrimMatchErr> {
    match optional_i64(winning_team, "winning_team")? {
        Some(0) => Ok(Some(team_a_id)),
        Some(1) => Ok(Some(team_b_id)),
        Some(winning_team) => Err(ScrimMatchErr::InvalidWinningTeam { winning_team }),
        None => Ok(None),
    }
}

fn optional_i64(value: Option<&Value>, field: &'static str) -> Result<Option<i64>, ScrimMatchErr> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64))
            .map(Some)
            .ok_or(ScrimMatchErr::InvalidTaskResultInt { field }),
        Some(Value::String(value)) => value
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| ScrimMatchErr::InvalidTaskResultInt { field }),
        _ => Err(ScrimMatchErr::InvalidTaskResultInt { field }),
    }
}

fn to_i32_id(label: &'static str, value: i64) -> Result<i32, ScrimMatchErr> {
    i32::try_from(value).map_err(|source| ScrimMatchErr::IdOutOfRange {
        label,
        value,
        source,
    })
}
