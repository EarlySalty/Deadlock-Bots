//! Minimaler Coach-Blick auf `scrim.*`.

use std::collections::{BTreeSet, HashMap};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use crate::db::{advisory_lock, i64_to_i32, unix_to_utc, utc_to_json_unix, DashboardDbResult};
use crate::web::{err_text, ok_json, DashboardApp};

const MATCHES_LOCK: i64 = 42_060_004_003;
const STATE_DRAFT: &str = "draft";
const STATE_SCHEDULED: &str = "scheduled";
const STATE_LOBBY_OPEN: &str = "lobby_open";
const STATE_RESULT_REQUESTED: &str = "result_requested";
const MATCH_REQUEST_DEFAULT_DEADLINE_HOURS: i64 = 48;
const MATCH_REQUEST_MIN_SLOTS: usize = 2;
const MATCH_REQUEST_MAX_SLOTS: usize = 5;

pub async fn scrims_overview(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    match load_overview(app.pool()).await {
        Ok(data) => ok_json(data),
        Err(err) => {
            tracing::error!(%err, "scrims_overview fehlgeschlagen");
            err_text(500, "Scrims unavailable")
        }
    }
}

pub async fn scrims_create_match(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let input = match parse_create_match(&payload) {
        Ok(input) => input,
        Err(resp) => return resp,
    };
    if input.team_a_id == input.team_b_id {
        return err_text(400, "team_a_id and team_b_id must differ");
    }
    for team_id in [input.team_a_id, input.team_b_id] {
        match team_exists(app.pool(), team_id).await {
            Ok(true) => {}
            Ok(false) => return err_text(400, &format!("team_id {team_id} does not exist")),
            Err(err) => {
                tracing::error!(%err, team_id, "Scrim-Team-Validierung fehlgeschlagen");
                return err_text(500, "Team validation failed");
            }
        }
    }

    match create_match_record(app.pool(), input).await {
        Ok(scrim_match) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match.create",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                match_id = scrim_match["id"].as_i64().unwrap_or_default(),
                "AUDIT scrims"
            );
            ok_json(json!({ "match": scrim_match }))
        }
        Err(err) => {
            tracing::error!(%err, "Scrim-Match-Anlage fehlgeschlagen");
            err_text(500, "Create match failed")
        }
    }
}

pub async fn scrims_match_request_defaults(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    ok_json(match_request_defaults_json())
}

pub async fn scrims_create_match_request_batch(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let input = match parse_match_request_batch(&payload, Utc::now()) {
        Ok(input) => input,
        Err(resp) => return resp,
    };
    match create_match_request_batch_record(
        app.pool(),
        &session.user_id.to_string(),
        &session.display_name,
        input,
    )
    .await
    {
        Ok(batch) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match_request.create",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                batch_id = batch["id"].as_i64().unwrap_or_default(),
                "AUDIT scrims"
            );
            ok_json(json!({ "batch": batch }))
        }
        Err(MatchRequestCreateError::BadRequest(message)) => err_text(400, message),
        Err(MatchRequestCreateError::Db(err)) => {
            tracing::error!(%err, "Scrim-Match-Abfrage-Anlage fehlgeschlagen");
            err_text(500, "Create match request failed")
        }
    }
}

pub async fn scrims_start_match(
    State(app): State<DashboardApp>,
    _headers: HeaderMap,
    Path(_match_id): Path<String>,
) -> Response {
    let _ = app;
    err_text(
        409,
        "Automatische Lobby-Erstellung ist deaktiviert; Lobbycode im Dashboard setzen.",
    )
}

pub async fn scrims_set_lobby_code(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(match_id): Path<String>,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let match_id = match parse_path_i32(&match_id, "match_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let code = match parse_lobby_code(&payload) {
        Ok(code) => code,
        Err(resp) => return resp,
    };

    match set_lobby_code(
        app.pool(),
        match_id,
        &code,
        &session.user_id.to_string(),
        &session.display_name,
    )
    .await
    {
        Ok(LobbyCodeUpdate::Updated(scrim_match)) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match.lobby_code",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                match_id,
                "AUDIT scrims"
            );
            ok_json(json!({ "match": scrim_match }))
        }
        Ok(LobbyCodeUpdate::NotFound) => err_text(404, "Match not found"),
        Ok(LobbyCodeUpdate::BotOwned(current)) => err_text(
            409,
            &format!(
                "Lobby state is controlled by bot: {}",
                current.unwrap_or_default()
            ),
        ),
        Err(err) => {
            tracing::error!(%err, match_id, "Scrim-Lobbycode-Speicherung fehlgeschlagen");
            err_text(500, "Save lobby code failed")
        }
    }
}

pub async fn scrims_request_result(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(match_id): Path<String>,
) -> Response {
    request_state(app, headers, &match_id, STATE_RESULT_REQUESTED).await
}

pub async fn scrims_update_participant_notes(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(participant_id): Path<String>,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let participant_id = match parse_path_i32(&participant_id, "participant_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let notes = match parse_notes(&payload) {
        Ok(notes) => notes,
        Err(resp) => return resp,
    };
    match update_participant_notes(app.pool(), participant_id, notes.clone()).await {
        Ok(true) => {
            tracing::info!(
                target: "audit",
                action = "scrim.participant.notes",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                participant_id,
                "AUDIT scrims"
            );
            ok_json(json!({ "participant_id": participant_id, "notes": notes }))
        }
        Ok(false) => err_text(404, "Participant not found"),
        Err(err) => {
            tracing::error!(%err, participant_id, "Scrim-Participant-Notiz fehlgeschlagen");
            err_text(500, "Save notes failed")
        }
    }
}

async fn request_state(
    app: DashboardApp,
    headers: HeaderMap,
    match_id: &str,
    requested_state: &'static str,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let match_id = match parse_path_i32(match_id, "match_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match set_lobby_request(app.pool(), match_id, requested_state).await {
        Ok(LobbyRequest::Updated(state)) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match.lobby_state",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                match_id,
                lobby_state = state,
                "AUDIT scrims"
            );
            ok_json(json!({ "match_id": match_id, "lobby_state": state }))
        }
        Ok(LobbyRequest::NotFound) => err_text(404, "Match not found"),
        Ok(LobbyRequest::BotOwned(current)) => err_text(
            409,
            &format!(
                "Lobby state is controlled by bot: {}",
                current.unwrap_or_default()
            ),
        ),
        Err(err) => {
            tracing::error!(%err, match_id, "Scrim-Lobby-State-Request fehlgeschlagen");
            err_text(500, "Lobby state update failed")
        }
    }
}

struct CreateMatchInput {
    team_a_id: i32,
    team_b_id: i32,
    scheduled_at: Option<DateTime<Utc>>,
    coach_spectator_discord_id: Option<i64>,
}

struct MatchRequestBatchInput {
    template: String,
    deadline_at: DateTime<Utc>,
    matches: Vec<MatchRequestInput>,
}

struct MatchRequestInput {
    team_a_id: i32,
    team_b_id: Option<i32>,
    slots: Vec<Value>,
}

#[derive(Debug, thiserror::Error)]
enum MatchRequestCreateError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn parse_create_match(payload: &Value) -> Result<CreateMatchInput, Response> {
    Ok(CreateMatchInput {
        team_a_id: parse_i32(get2(payload, "team_a_id", "teamAId"), "team_a_id")?,
        team_b_id: parse_i32(get2(payload, "team_b_id", "teamBId"), "team_b_id")?,
        scheduled_at: parse_optional_datetime(
            get2(payload, "scheduled_at", "scheduledAt"),
            "scheduled_at",
        )?,
        coach_spectator_discord_id: parse_optional_i64(
            get2(
                payload,
                "coach_spectator_discord_id",
                "coachSpectatorDiscordId",
            ),
            "coach_spectator_discord_id",
        )?,
    })
}

fn match_request_defaults_json() -> Value {
    json!({
        "default_deadline_hours": MATCH_REQUEST_DEFAULT_DEADLINE_HOURS,
        "min_slots": MATCH_REQUEST_MIN_SLOTS,
        "max_slots": MATCH_REQUEST_MAX_SLOTS,
        "templates": [
            {
                "key": "regular_scrim",
                "name": "Regulärer Scrim",
                "focus": "Vollständige Lineups und Slot-Findung"
            },
            {
                "key": "testmatch",
                "name": "Testmatch",
                "focus": "Bereitschaft und klares Abstimmungsergebnis"
            },
            {
                "key": "training",
                "name": "Training/Teamspiel",
                "focus": "Interne Aktivität und Zusammenspiel"
            }
        ],
        "presets": [{
            "key": "weekend_evening",
            "name": "Wochenende abends",
            "slots": [
                { "day": "sat", "from": 20 * 60, "to": 22 * 60 },
                { "day": "sun", "from": 20 * 60, "to": 22 * 60 }
            ]
        }]
    })
}

fn parse_match_request_batch(
    payload: &Value,
    now: DateTime<Utc>,
) -> Result<MatchRequestBatchInput, Response> {
    let template = match get2(payload, "template", "template") {
        Some(Value::String(value)) => value.trim().to_string(),
        _ => return Err(err_text(400, "template must be string")),
    };
    if !["regular_scrim", "testmatch", "training"].contains(&template.as_str()) {
        return Err(err_text(400, "Invalid match request template"));
    }
    let deadline_at =
        parse_optional_datetime(get2(payload, "deadline_at", "deadlineAt"), "deadline_at")?
            .unwrap_or_else(|| now + ChronoDuration::hours(MATCH_REQUEST_DEFAULT_DEADLINE_HOURS));
    if deadline_at <= now {
        return Err(err_text(400, "deadline_at must be in the future"));
    }
    let common_slots = match get2(payload, "slots", "slotOptions") {
        Some(raw) => Some(parse_match_request_slots(raw)?),
        None => None,
    };
    let raw_matches = match payload.get("matches") {
        Some(Value::Array(values)) if !values.is_empty() => values,
        _ => return Err(err_text(400, "matches must be a non-empty array")),
    };

    let mut seen_team_ids = BTreeSet::new();
    let mut matches = Vec::with_capacity(raw_matches.len());
    for raw in raw_matches {
        let Some(obj) = raw.as_object() else {
            return Err(err_text(400, "matches entries must be objects"));
        };
        let team_a_id = parse_i32(
            obj.get("team_a_id").or_else(|| obj.get("teamAId")),
            "team_a_id",
        )?;
        let team_b_id = parse_optional_i32(
            obj.get("team_b_id").or_else(|| obj.get("teamBId")),
            "team_b_id",
        )?;
        if team_a_id <= 0 || team_b_id.is_some_and(|id| id <= 0) {
            return Err(err_text(400, "team ids must be positive"));
        }
        if team_b_id == Some(team_a_id) {
            return Err(err_text(400, "team_a_id and team_b_id must differ"));
        }
        for team_id in [Some(team_a_id), team_b_id].into_iter().flatten() {
            if !seen_team_ids.insert(team_id) {
                return Err(err_text(
                    400,
                    "Ein Team darf im gleichen aktiven Abfragezeitraum nur in einem Match stecken.",
                ));
            }
        }
        let slots = match obj.get("slots").or_else(|| obj.get("slotOptions")) {
            Some(raw) => parse_match_request_slots(raw)?,
            None => common_slots
                .clone()
                .ok_or_else(|| err_text(400, "Each match needs two to five slots"))?,
        };
        matches.push(MatchRequestInput {
            team_a_id,
            team_b_id,
            slots,
        });
    }

    Ok(MatchRequestBatchInput {
        template,
        deadline_at,
        matches,
    })
}

fn parse_match_request_slots(raw: &Value) -> Result<Vec<Value>, Response> {
    let Some(items) = raw.as_array() else {
        return Err(err_text(400, "slots must be an array"));
    };
    if !(MATCH_REQUEST_MIN_SLOTS..=MATCH_REQUEST_MAX_SLOTS).contains(&items.len()) {
        return Err(err_text(400, "Each match needs two to five slots"));
    }
    items
        .iter()
        .map(|item| {
            let Some(obj) = item.as_object() else {
                return Err(err_text(400, "slot entries must be objects"));
            };
            let day = match obj.get("day") {
                Some(Value::String(day)) => day.trim().to_ascii_lowercase(),
                _ => return Err(err_text(400, "slot day must be string")),
            };
            if !matches!(
                day.as_str(),
                "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun"
            ) {
                return Err(err_text(400, "slot day is invalid"));
            }
            let from = parse_i32(obj.get("from"), "from")?;
            let to = parse_i32(obj.get("to"), "to")?;
            if !(0 <= from && from < to && to <= 1440) {
                return Err(err_text(400, "slot time window is invalid"));
            }
            Ok(json!({ "day": day, "from": from, "to": to }))
        })
        .collect()
}

fn get2<'a>(obj: &'a Value, k1: &str, k2: &str) -> Option<&'a Value> {
    obj.get(k1).or_else(|| obj.get(k2))
}

fn parse_i32(raw: Option<&Value>, field: &'static str) -> Result<i32, Response> {
    let value = parse_i64(raw, field)?;
    i64_to_i32(value, field).map_err(|_| err_text(400, &format!("{field} must fit integer")))
}

fn parse_i64(raw: Option<&Value>, field: &str) -> Result<i64, Response> {
    match raw {
        Some(Value::Number(n)) => n
            .as_i64()
            .ok_or_else(|| err_text(400, &format!("{field} must be integer"))),
        Some(Value::String(s)) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| err_text(400, &format!("{field} must be integer"))),
        _ => Err(err_text(400, &format!("{field} must be integer"))),
    }
}

fn parse_optional_i64(raw: Option<&Value>, field: &str) -> Result<Option<i64>, Response> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(_) => parse_i64(raw, field).map(Some),
    }
}

fn parse_optional_i32(raw: Option<&Value>, field: &'static str) -> Result<Option<i32>, Response> {
    parse_optional_i64(raw, field).and_then(|value| match value {
        Some(value) => i64_to_i32(value, field)
            .map(Some)
            .map_err(|_| err_text(400, &format!("{field} must fit integer"))),
        None => Ok(None),
    })
}

fn parse_path_i32(raw: &str, field: &'static str) -> Result<i32, Response> {
    let value = raw
        .trim()
        .parse::<i64>()
        .map_err(|_| err_text(400, &format!("{field} must be integer")))?;
    i64_to_i32(value, field).map_err(|_| err_text(400, &format!("{field} must fit integer")))
}

fn parse_optional_datetime(
    raw: Option<&Value>,
    field: &str,
) -> Result<Option<DateTime<Utc>>, Response> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::Number(n)) => {
            let ts = n
                .as_i64()
                .ok_or_else(|| err_text(400, &format!("{field} must be unix seconds")))?;
            unix_to_utc(ts)
                .map(Some)
                .map_err(|_| err_text(400, &format!("{field} is out of range")))
        }
        Some(Value::String(s)) => {
            let value = s.trim();
            if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
                return Ok(Some(dt.with_timezone(&Utc)));
            }
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
                .map(|dt| Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc)))
                .map_err(|_| err_text(400, &format!("{field} must be RFC3339 or unix seconds")))
        }
        Some(_) => Err(err_text(400, &format!("{field} must be datetime"))),
    }
}

fn parse_notes(payload: &Value) -> Result<Option<String>, Response> {
    match payload.get("notes").or_else(|| payload.get("note")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => {
            if s.chars().count() > 4000 {
                return Err(err_text(400, "notes must be at most 4000 characters"));
            }
            Ok(Some(s.to_string()))
        }
        Some(_) => Err(err_text(400, "notes must be string")),
    }
}

fn parse_lobby_code(payload: &Value) -> Result<String, Response> {
    let raw = match get2(payload, "code", "lobbyCode") {
        Some(Value::String(value)) => value.trim(),
        _ => return Err(err_text(400, "code must be string")),
    };
    if raw.chars().count() != 5 || !raw.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(err_text(400, "code must be exactly 5 letters or numbers"));
    }
    Ok(raw.to_ascii_uppercase())
}

async fn load_overview(pool: &PgPool) -> DashboardDbResult<Value> {
    let participants = load_participants(pool).await?;
    let teams = load_teams(pool).await?;
    let matches = load_matches(pool).await?;
    Ok(json!({
        "teams": teams,
        "participants": participants,
        "matches": matches,
    }))
}

async fn load_participants(pool: &PgPool) -> DashboardDbResult<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT id::bigint AS id,
               discord_id,
               display_name,
               rank,
               roles,
               availability,
               availability_slots,
               notes,
               status
          FROM scrim.participants
         ORDER BY status ASC, display_name ASC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(json!({
                "id": row.try_get::<i64, _>("id")?,
                "discord_id": row.try_get::<Option<i64>, _>("discord_id")?,
                "display_name": row.try_get::<String, _>("display_name")?,
                "rank": row.try_get::<Option<String>, _>("rank")?,
                "roles": row.try_get::<Option<String>, _>("roles")?,
                "availability": row.try_get::<Option<String>, _>("availability")?,
                "availability_slots": row.try_get::<Option<Value>, _>("availability_slots")?,
                "notes": row.try_get::<Option<String>, _>("notes")?,
                "status": row.try_get::<String, _>("status")?,
            }))
        })
        .collect()
}

async fn load_teams(pool: &PgPool) -> DashboardDbResult<Vec<Value>> {
    let member_rows = sqlx::query(
        r#"
        SELECT tm.team_id::bigint AS team_id,
               p.id::bigint AS participant_id,
               p.display_name,
               p.rank,
               p.discord_id,
               tm.role,
               tm.is_captain,
               tm.is_bench
          FROM scrim.team_members tm
          JOIN scrim.participants p ON p.id = tm.participant_id
         ORDER BY tm.team_id ASC, tm.is_bench ASC, tm.role ASC, p.display_name ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut members: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in member_rows {
        let team_id = row.try_get::<i64, _>("team_id")?;
        members.entry(team_id).or_default().push(json!({
            "participant_id": row.try_get::<i64, _>("participant_id")?,
            "display_name": row.try_get::<String, _>("display_name")?,
            "rank": row.try_get::<Option<String>, _>("rank")?,
            "discord_id": row.try_get::<Option<i64>, _>("discord_id")?,
            "role": row.try_get::<Option<String>, _>("role")?,
            "is_captain": row.try_get::<bool, _>("is_captain")?,
            "is_bench": row.try_get::<bool, _>("is_bench")?,
        }));
    }

    let rows = sqlx::query(
        r#"
        SELECT id::bigint AS id,
               name,
               coach,
               discord_role_id,
               discord_channel_id
          FROM scrim.teams
         ORDER BY name ASC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut teams = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row.try_get::<i64, _>("id")?;
        teams.push(json!({
            "id": id,
            "name": row.try_get::<String, _>("name")?,
            "coach": row.try_get::<Option<String>, _>("coach")?,
            "discord_role_id": row.try_get::<Option<i64>, _>("discord_role_id")?,
            "discord_channel_id": row.try_get::<Option<i64>, _>("discord_channel_id")?,
            "members": members.remove(&id).unwrap_or_default(),
        }));
    }
    Ok(teams)
}

async fn load_matches(pool: &PgPool) -> DashboardDbResult<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT m.id::bigint AS id,
               m.team_a_id::bigint AS team_a_id,
               ta.name AS team_a_name,
               m.team_b_id::bigint AS team_b_id,
               tb.name AS team_b_name,
               m.when_text,
               m.scheduled_at,
               m.status,
	               m.lobby_state,
	               m.join_code,
	               m.lobby_code_source_user_id,
	               m.lobby_code_source_display_name,
	               m.lobby_code_updated_at,
	               m.lobby_code_message_ids,
	               m.lobby_code_corrections,
	               m.steam_match_id,
	               m.winner_team_id::bigint AS winner_team_id,
	               m.coach_spectator_discord_id,
	               m.created_at,
               m.updated_at
          FROM scrim.matches m
          LEFT JOIN scrim.teams ta ON ta.id = m.team_a_id
          LEFT JOIN scrim.teams tb ON tb.id = m.team_b_id
         ORDER BY m.scheduled_at DESC NULLS LAST, m.created_at DESC, m.id DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(match_json).collect()
}

fn match_json(row: sqlx::postgres::PgRow) -> DashboardDbResult<Value> {
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "team_a_id": row.try_get::<Option<i64>, _>("team_a_id")?,
        "team_a_name": row.try_get::<Option<String>, _>("team_a_name")?,
        "team_b_id": row.try_get::<Option<i64>, _>("team_b_id")?,
        "team_b_name": row.try_get::<Option<String>, _>("team_b_name")?,
        "when_text": row.try_get::<Option<String>, _>("when_text")?,
        "scheduled_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("scheduled_at")?),
            "status": row.try_get::<String, _>("status")?,
            "lobby_state": row.try_get::<Option<String>, _>("lobby_state")?,
            "join_code": row.try_get::<Option<String>, _>("join_code")?,
            "lobby_code_source_user_id": row.try_get::<Option<String>, _>("lobby_code_source_user_id")?,
            "lobby_code_source_display_name": row.try_get::<Option<String>, _>("lobby_code_source_display_name")?,
            "lobby_code_updated_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("lobby_code_updated_at")?),
            "lobby_code_message_ids": row.try_get::<Option<Value>, _>("lobby_code_message_ids")?.unwrap_or_else(|| json!({})),
            "lobby_code_corrections": row.try_get::<Option<Value>, _>("lobby_code_corrections")?.unwrap_or_else(|| json!([])),
            "steam_match_id": row.try_get::<Option<i64>, _>("steam_match_id")?,
            "winner_team_id": row.try_get::<Option<i64>, _>("winner_team_id")?,
            "coach_spectator_discord_id": row.try_get::<Option<i64>, _>("coach_spectator_discord_id")?,
            "created_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("created_at")?),
        "updated_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("updated_at")?),
    }))
}

async fn team_exists(pool: &PgPool, team_id: i32) -> DashboardDbResult<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(SELECT 1 FROM scrim.teams WHERE id = $1)
        "#,
    )
    .bind(team_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

async fn create_match_record(pool: &PgPool, input: CreateMatchInput) -> DashboardDbResult<Value> {
    let mut tx = pool.begin().await?;
    advisory_lock(&mut tx, MATCHES_LOCK).await?;
    let id = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int4
          FROM scrim.matches
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.matches(
            id, team_a_id, team_b_id, scheduled_at, status, lobby_state,
            coach_spectator_discord_id, created_at, updated_at
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, now(), now())
        "#,
    )
    .bind(id)
    .bind(input.team_a_id)
    .bind(input.team_b_id)
    .bind(input.scheduled_at)
    .bind(STATE_SCHEDULED)
    .bind(STATE_DRAFT)
    .bind(input.coach_spectator_discord_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    load_match(pool, id)
        .await?
        .ok_or(sqlx::Error::RowNotFound.into())
}

async fn create_match_request_batch_record(
    pool: &PgPool,
    created_by_user_id: &str,
    created_by_display_name: &str,
    input: MatchRequestBatchInput,
) -> Result<Value, MatchRequestCreateError> {
    let team_ids = input
        .matches
        .iter()
        .flat_map(|request| [Some(request.team_a_id), request.team_b_id])
        .flatten()
        .collect::<Vec<_>>();
    let mut tx = pool.begin().await?;
    advisory_lock(&mut tx, MATCHES_LOCK).await?;

    let existing_team_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scrim.teams WHERE id = ANY($1)")
            .bind(&team_ids)
            .fetch_one(&mut *tx)
            .await?;
    if existing_team_count != team_ids.len() as i64 {
        return Err(MatchRequestCreateError::BadRequest(
            "At least one team was not found",
        ));
    }

    let active_conflict: Option<i32> = sqlx::query_scalar(
        r#"
        SELECT active.team_id
          FROM (
                SELECT mr.team_a_id AS team_id
                  FROM scrim.match_requests mr
                  JOIN scrim.match_request_batches b ON b.id = mr.batch_id
                 WHERE b.status IN ('draft', 'posting', 'open', 'post_failed')
                   AND b.deadline_at > now()
                UNION ALL
                SELECT mr.team_b_id AS team_id
                  FROM scrim.match_requests mr
                  JOIN scrim.match_request_batches b ON b.id = mr.batch_id
                 WHERE mr.team_b_id IS NOT NULL
                   AND b.status IN ('draft', 'posting', 'open', 'post_failed')
                   AND b.deadline_at > now()
          ) active
         WHERE active.team_id = ANY($1)
         LIMIT 1
        "#,
    )
    .bind(&team_ids)
    .fetch_optional(&mut *tx)
    .await?;
    if active_conflict.is_some() {
        return Err(MatchRequestCreateError::BadRequest(
            "Ein Team darf im gleichen aktiven Abfragezeitraum nur in einem Match stecken.",
        ));
    }

    let batch_id = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int4
          FROM scrim.match_request_batches
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.match_request_batches(
            id, template, deadline_at, status, created_by_user_id,
            created_by_display_name, created_at, updated_at
        )
        VALUES($1, $2, $3, 'draft', $4, $5, now(), now())
        "#,
    )
    .bind(batch_id)
    .bind(&input.template)
    .bind(input.deadline_at)
    .bind(created_by_user_id)
    .bind(created_by_display_name)
    .execute(&mut *tx)
    .await?;

    let mut matches = Vec::with_capacity(input.matches.len());
    for request in input.matches {
        let request_id = sqlx::query_scalar::<_, i32>(
            r#"
            SELECT (COALESCE(MAX(id), 0) + 1)::int4
              FROM scrim.match_requests
            "#,
        )
        .fetch_one(&mut *tx)
        .await?;
        let slot_options = Value::Array(request.slots.clone());
        sqlx::query(
            r#"
            INSERT INTO scrim.match_requests(
                id, batch_id, team_a_id, team_b_id, status, slot_options, created_at, updated_at
            )
            VALUES($1, $2, $3, $4, 'draft', $5::jsonb, now(), now())
            "#,
        )
        .bind(request_id)
        .bind(batch_id)
        .bind(request.team_a_id)
        .bind(request.team_b_id)
        .bind(&slot_options)
        .execute(&mut *tx)
        .await?;
        matches.push(json!({
            "id": request_id,
            "team_a_id": request.team_a_id,
            "team_b_id": request.team_b_id,
            "status": "draft",
            "slots": slot_options,
        }));
    }
    tx.commit().await?;

    Ok(json!({
        "id": batch_id,
        "status": "draft",
        "template": input.template,
        "deadline_at": utc_to_json_unix(Some(input.deadline_at)),
        "matches": matches,
    }))
}

async fn load_match(pool: &PgPool, id: i32) -> DashboardDbResult<Option<Value>> {
    let row = sqlx::query(
        r#"
        SELECT m.id::bigint AS id,
               m.team_a_id::bigint AS team_a_id,
               ta.name AS team_a_name,
               m.team_b_id::bigint AS team_b_id,
               tb.name AS team_b_name,
               m.when_text,
               m.scheduled_at,
               m.status,
	               m.lobby_state,
	               m.join_code,
	               m.lobby_code_source_user_id,
	               m.lobby_code_source_display_name,
	               m.lobby_code_updated_at,
	               m.lobby_code_message_ids,
	               m.lobby_code_corrections,
	               m.steam_match_id,
	               m.winner_team_id::bigint AS winner_team_id,
	               m.coach_spectator_discord_id,
	               m.created_at,
               m.updated_at
          FROM scrim.matches m
          LEFT JOIN scrim.teams ta ON ta.id = m.team_a_id
          LEFT JOIN scrim.teams tb ON tb.id = m.team_b_id
         WHERE m.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(match_json).transpose()
}

enum LobbyRequest {
    Updated(&'static str),
    NotFound,
    BotOwned(Option<String>),
}

enum LobbyCodeUpdate {
    Updated(Value),
    NotFound,
    BotOwned(Option<String>),
}

async fn set_lobby_code(
    pool: &PgPool,
    match_id: i32,
    code: &str,
    source_user_id: &str,
    source_display_name: &str,
) -> DashboardDbResult<LobbyCodeUpdate> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query(
        r#"
        SELECT lobby_state, join_code
          FROM scrim.matches
         WHERE id = $1
         FOR UPDATE
        "#,
    )
    .bind(match_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = current else {
        return Ok(LobbyCodeUpdate::NotFound);
    };
    let lobby_state = row.try_get::<Option<String>, _>("lobby_state")?;
    if lobby_state.as_deref().is_some_and(is_bot_owned_lobby_state) {
        return Ok(LobbyCodeUpdate::BotOwned(lobby_state));
    }

    let previous_code = row.try_get::<Option<String>, _>("join_code")?;
    let corrections = previous_code
        .as_deref()
        .filter(|previous| *previous != code)
        .map(|previous| {
            json!([{
                "from": previous,
                "to": code,
                "source_user_id": source_user_id,
                "source_display_name": source_display_name,
                "at": Utc::now().timestamp(),
            }])
        })
        .unwrap_or_else(|| json!([]));

    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET join_code = $2,
               lobby_state = $3,
               lobby_code_source_user_id = $4,
               lobby_code_source_display_name = $5,
               lobby_code_updated_at = now(),
               lobby_code_corrections = COALESCE(lobby_code_corrections, '[]'::jsonb) || $6::jsonb,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(match_id)
    .bind(code)
    .bind(STATE_LOBBY_OPEN)
    .bind(source_user_id)
    .bind(source_display_name)
    .bind(&corrections)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(LobbyCodeUpdate::Updated(
        load_match(pool, match_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?,
    ))
}

async fn set_lobby_request(
    pool: &PgPool,
    match_id: i32,
    requested_state: &'static str,
) -> DashboardDbResult<LobbyRequest> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query(
        r#"
        SELECT lobby_state
          FROM scrim.matches
         WHERE id = $1
         FOR UPDATE
        "#,
    )
    .bind(match_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = current else {
        return Ok(LobbyRequest::NotFound);
    };
    let lobby_state = row.try_get::<Option<String>, _>("lobby_state")?;
    if lobby_state.as_deref().is_some_and(is_bot_owned_lobby_state) {
        return Ok(LobbyRequest::BotOwned(lobby_state));
    }
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET lobby_state = $2,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(match_id)
    .bind(requested_state)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(LobbyRequest::Updated(requested_state))
}

fn is_bot_owned_lobby_state(state: &str) -> bool {
    matches!(
        state,
        "start_requested"
            | "lobby_open"
            | "starting"
            | "lobby_posting"
            | "result_requested"
            | "start_failed"
            | "in_progress"
            | "finished"
            | "result_fetching"
            | "result_failed"
    )
}

async fn update_participant_notes(
    pool: &PgPool,
    participant_id: i32,
    notes: Option<String>,
) -> DashboardDbResult<bool> {
    let changed = sqlx::query(
        r#"
        UPDATE scrim.participants
           SET notes = $2,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(participant_id)
    .bind(notes)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::authority::{MemberAccessInfo, MemberLookup};
    use crate::config::{AccessLevel, DashboardConfig};
    use crate::names::NameResolver;
    use crate::now_unix_f64;
    use crate::web::{router, SESSION_COOKIE};

    struct NoMemberLookup;

    #[async_trait::async_trait]
    impl MemberLookup for NoMemberLookup {
        async fn member_access(
            &self,
            _guild_id: Option<u64>,
            _user_id: u64,
        ) -> Option<MemberAccessInfo> {
            None
        }
    }

    struct NoNameResolver;

    #[async_trait::async_trait]
    impl NameResolver for NoNameResolver {
        async fn resolve(&self, _user_ids: &[u64]) -> HashMap<u64, String> {
            HashMap::new()
        }
    }

    async fn app_with_session(
    ) -> Result<(dl_central_db::TestDb, axum::Router, String, String), Box<dyn std::error::Error>>
    {
        let db = dl_central_db::testing::test_pool().await?;
        let session_id = "scrim-test-session".to_string();
        let csrf = "scrim-test-csrf".to_string();
        let now = now_unix_f64();
        dl_central_db::kv::set(
            db.pool(),
            "dl_dashboard_admin_session",
            &session_id,
            &json!({
                "user_id": 42,
                "username": "coach",
                "display_name": "Coach",
                "reason": "test",
                "access_level": AccessLevel::Full.as_str(),
                "csrf_token": csrf,
                "created_at": now,
                "last_seen_at": now,
                "expires_at": now + 3600.0,
            })
            .to_string(),
        )
        .await?;
        let cfg = DashboardConfig::from_lookup(|key| match key {
            "DISCORD_OAUTH_CLIENT_ID" => Some("id".to_string()),
            "DISCORD_OAUTH_CLIENT_SECRET" => Some("secret".to_string()),
            _ => None,
        });
        let app = DashboardApp::new(
            cfg,
            db.pool().clone(),
            Arc::new(NoMemberLookup),
            Arc::new(NoNameResolver),
        )
        .await?;
        Ok((db, router(app), session_id, csrf))
    }

    fn auth_post(
        uri: &str,
        session_id: &str,
        csrf: &str,
        body: Value,
    ) -> Result<Request<Body>, axum::http::Error> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::ORIGIN,
                "https://admin.deutsche-deadlock-community.de",
            )
            .header("X-CSRF-Token", csrf)
            .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
            .body(Body::from(body.to_string()))
    }

    fn auth_get(uri: &str, session_id: &str) -> Result<Request<Body>, axum::http::Error> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
            .body(Body::empty())
    }

    async fn insert_team(pool: &PgPool, id: i32, name: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.teams(id, name, created_at)
            VALUES($1, $2, now())
            "#,
        )
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match(pool: &PgPool, id: i32, state: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.matches(
                id, team_a_id, team_b_id, status, lobby_state, created_at, updated_at
            )
            VALUES($1, 1, 2, 'scheduled', $2, now(), now())
            "#,
        )
        .bind(id)
        .bind(state)
        .execute(pool)
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn create_match_route_insertet_scheduled_draft() -> Result<(), Box<dyn std::error::Error>>
    {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/matches",
                &session_id,
                &csrf,
                json!({
                    "team_a_id": 1,
                    "team_b_id": 2,
                    "coach_spectator_discord_id": "123456789",
                    "scheduled_at": "2026-07-06T19:00:00Z",
                }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let match_id = data["match"]["id"].as_i64().ok_or("missing match id")?;
        let row = sqlx::query(
            r#"
            SELECT team_a_id, team_b_id, status, lobby_state, coach_spectator_discord_id
              FROM scrim.matches
             WHERE id = $1
            "#,
        )
        .bind(i64_to_i32(match_id, "match_id")?)
        .fetch_one(db.pool())
        .await?;

        assert_eq!(row.try_get::<i32, _>("team_a_id")?, 1);
        assert_eq!(row.try_get::<i32, _>("team_b_id")?, 2);
        assert_eq!(row.try_get::<String, _>("status")?, "scheduled");
        assert_eq!(
            row.try_get::<Option<String>, _>("lobby_state")?,
            Some("draft".to_string())
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("coach_spectator_discord_id")?,
            Some(123456789)
        );
        Ok(())
    }

    #[tokio::test]
    async fn match_request_defaults_route_liefert_zielsystem_defaults(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, session_id, _csrf) = app_with_session().await?;

        let response = app
            .oneshot(auth_get(
                "/api/scrims/match-requests/defaults",
                &session_id,
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["default_deadline_hours"], 48);
        assert_eq!(data["min_slots"], 2);
        assert_eq!(data["max_slots"], 5);
        assert_eq!(data["templates"][0]["key"], "regular_scrim");
        assert_eq!(data["templates"][1]["key"], "testmatch");
        assert_eq!(data["templates"][2]["key"], "training");
        assert_eq!(data["presets"][0]["slots"][0]["day"], "sat");
        assert_eq!(data["presets"][0]["slots"][1]["day"], "sun");
        Ok(())
    }

    #[tokio::test]
    async fn create_match_request_batch_speichert_slots_frist_template_und_blockiert_team_doppelplanung(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team(db.pool(), 3, "C").await?;

        let response = app
            .clone()
            .oneshot(auth_post(
                "/api/scrims/match-requests",
                &session_id,
                &csrf,
                json!({
                    "template": "regular_scrim",
                    "slots": [
                        { "day": "sat", "from": 20 * 60, "to": 22 * 60 },
                        { "day": "sun", "from": 20 * 60, "to": 22 * 60 }
                    ],
                    "matches": [
                        { "team_a_id": 1, "team_b_id": 2 },
                        { "team_a_id": 3, "team_b_id": null }
                    ]
                }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["batch"]["status"], "draft");
        assert_eq!(data["batch"]["template"], "regular_scrim");
        assert_eq!(
            data["batch"]["matches"].as_array().ok_or("matches")?.len(),
            2
        );
        assert_eq!(data["batch"]["matches"][0]["slots"][0]["day"], "sat");
        assert_eq!(data["batch"]["matches"][1]["team_b_id"], Value::Null);

        let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scrim.match_requests")
            .fetch_one(db.pool())
            .await?;
        assert_eq!(stored, 2);

        let duplicate = app
            .oneshot(auth_post(
                "/api/scrims/match-requests",
                &session_id,
                &csrf,
                json!({
                    "template": "regular_scrim",
                    "slots": [
                        { "day": "sat", "from": 20 * 60, "to": 22 * 60 },
                        { "day": "sun", "from": 20 * 60, "to": 22 * 60 }
                    ],
                    "matches": [{ "team_a_id": 2, "team_b_id": null }]
                }),
            )?)
            .await?;
        assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn start_route_startet_keine_alte_vollautomatik() -> Result<(), Box<dyn std::error::Error>>
    {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 10, "draft").await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/matches/10/start",
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let state =
            sqlx::query_scalar::<_, String>("SELECT lobby_state FROM scrim.matches WHERE id = 10")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(state, "draft");
        Ok(())
    }

    #[tokio::test]
    async fn result_route_ueberschreibt_lobby_open_nicht() -> Result<(), Box<dyn std::error::Error>>
    {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 12, "lobby_open").await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/matches/12/result",
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let state =
            sqlx::query_scalar::<_, String>("SELECT lobby_state FROM scrim.matches WHERE id = 12")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(state, "lobby_open");
        Ok(())
    }

    #[tokio::test]
    async fn result_route_ueberschreibt_parallelen_lobby_open_nicht(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 13, "draft").await?;

        let mut tx = db.pool().begin().await?;
        sqlx::query("SELECT id FROM scrim.matches WHERE id = 13 FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;

        let pending_result = tokio::spawn({
            let app = app.clone();
            let session_id = session_id.clone();
            let csrf = csrf.clone();
            async move {
                app.oneshot(
                    auth_post(
                        "/api/scrims/matches/13/result",
                        &session_id,
                        &csrf,
                        json!({}),
                    )
                    .expect("request"),
                )
                .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        sqlx::query("UPDATE scrim.matches SET lobby_state = 'lobby_open' WHERE id = 13")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        let response =
            tokio::time::timeout(std::time::Duration::from_secs(5), pending_result).await???;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let state =
            sqlx::query_scalar::<_, String>("SELECT lobby_state FROM scrim.matches WHERE id = 13")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(state, "lobby_open");
        Ok(())
    }

    #[tokio::test]
    async fn lobby_code_route_normalisiert_speichert_und_validiert_zielsystem_code(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 10, "draft").await?;

        let response = app
            .clone()
            .oneshot(auth_post(
                "/api/scrims/matches/10/lobby-code",
                &session_id,
                &csrf,
                json!({ "code": "abC12" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let row = sqlx::query("SELECT join_code, lobby_state FROM scrim.matches WHERE id = 10")
            .fetch_one(db.pool())
            .await?;
        assert_eq!(
            row.try_get::<Option<String>, _>("join_code")?,
            Some("ABC12".to_string())
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("lobby_state")?,
            Some("lobby_open".to_string())
        );

        let response = app
            .clone()
            .oneshot(auth_post(
                "/api/scrims/matches/10/lobby-code",
                &session_id,
                &csrf,
                json!({ "code": "ABC-1" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        insert_match(db.pool(), 11, "result_requested").await?;
        let response = app
            .oneshot(auth_post(
                "/api/scrims/matches/11/lobby-code",
                &session_id,
                &csrf,
                json!({ "code": "XYZ99" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let state =
            sqlx::query_scalar::<_, String>("SELECT lobby_state FROM scrim.matches WHERE id = 11")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(state, "result_requested");
        Ok(())
    }
}
