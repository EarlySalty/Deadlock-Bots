//! Minimaler Coach-Blick auf `scrim.*`.

use std::collections::{BTreeSet, HashMap};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, Utc};
use dl_ai::ChatProviderError;
use dl_central_db::scrim_runtime::require_local_scrim_write;
use dl_squads::lagebild::{
    revise_lagebild, LagebildError, ScrimLagebildEvidence, MAIN_GUILD_ID as SCRIM_MAIN_GUILD_ID,
};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool, Row};

use crate::db::{
    advisory_lock, i64_to_i32, unix_to_utc, utc_to_json_unix, DashboardDbError, DashboardDbResult,
};
use crate::web::{err_text, ok_json, DashboardApp};

const MATCHES_LOCK: i64 = 42_060_004_003;
const STATE_DRAFT: &str = "draft";
const STATE_SCHEDULED: &str = "scheduled";
const STATE_LOBBY_OPEN: &str = "lobby_open";
const STATE_RESULT_REQUESTED: &str = "result_requested";
const MATCH_REQUEST_DEFAULT_DEADLINE_HOURS: i64 = 48;
const MATCH_REQUEST_MIN_SLOTS: usize = 2;
const MATCH_REQUEST_MAX_SLOTS: usize = 5;
const SCRIM_RUNTIME_DENIED_MESSAGE: &str =
    "Die Scrim-Verwaltung wird gerade umgestellt. Änderungen am Roster sind über dieses Dashboard vorübergehend nicht möglich.";
const MATCH_REQUEST_SUMMARY_LIMIT: i64 = 10;
const MATCH_REQUEST_REMINDER_DEFAULT_TEMPLATE: &str = "antwort_fehlt";
const MATCH_REQUEST_REMINDER_TEMPLATES: [&str; 3] =
    ["antwort_fehlt", "frist_bald", "bestaetigung_offen"];
const MATCH_REQUEST_REPLACEMENT_DATA_NOTE: &str =
    "Rollen/Lineup-Daten fehlen; angezeigt werden nur konkrete Personen aus Teammitgliedern und Antworten.";

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
    match load_slot_presets(app.pool()).await {
        Ok(presets) => ok_json(match_request_defaults_json(presets)),
        Err(err) => {
            tracing::error!(%err, "Scrim-Slot-Presets konnten nicht geladen werden");
            err_text(500, "Scrim slot presets unavailable")
        }
    }
}

pub async fn scrims_create_slot_preset(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let input = match parse_slot_preset_body(&body) {
        Ok(input) => input,
        Err(resp) => return resp,
    };
    match create_slot_preset(app.pool(), &session.user_id.to_string(), input).await {
        Ok(preset) => {
            tracing::info!(
                target: "audit",
                action = "scrim.slot_preset.create",
                user_id = session.user_id,
                display_name = %session.display_name,
                preset_id = preset["id"].as_i64().unwrap_or_default(),
                "AUDIT scrims"
            );
            ok_json(json!({ "preset": preset }))
        }
        Err(err) => {
            tracing::error!(%err, "Scrim-Slot-Preset konnte nicht angelegt werden");
            err_text(500, "Create scrim slot preset failed")
        }
    }
}

pub async fn scrims_update_slot_preset(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(preset_id): Path<String>,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let preset_id = match parse_path_i64(&preset_id, "preset_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let input = match parse_slot_preset_body(&body) {
        Ok(input) => input,
        Err(resp) => return resp,
    };
    match update_slot_preset(app.pool(), preset_id, input).await {
        Ok(Some(preset)) => {
            tracing::info!(
                target: "audit",
                action = "scrim.slot_preset.update",
                user_id = session.user_id,
                display_name = %session.display_name,
                preset_id,
                "AUDIT scrims"
            );
            ok_json(json!({ "preset": preset }))
        }
        Ok(None) => err_text(404, "Scrim slot preset not found"),
        Err(err) => {
            tracing::error!(%err, preset_id, "Scrim-Slot-Preset konnte nicht geändert werden");
            err_text(500, "Update scrim slot preset failed")
        }
    }
}

pub async fn scrims_delete_slot_preset(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(preset_id): Path<String>,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let preset_id = match parse_path_i64(&preset_id, "preset_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match delete_slot_preset(app.pool(), preset_id).await {
        Ok(true) => {
            tracing::info!(
                target: "audit",
                action = "scrim.slot_preset.delete",
                user_id = session.user_id,
                display_name = %session.display_name,
                preset_id,
                "AUDIT scrims"
            );
            ok_json(json!({ "deleted": true, "id": preset_id }))
        }
        Ok(false) => err_text(404, "Scrim slot preset not found"),
        Err(err) => {
            tracing::error!(%err, preset_id, "Scrim-Slot-Preset konnte nicht gelöscht werden");
            err_text(500, "Delete scrim slot preset failed")
        }
    }
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

pub async fn scrims_match_request_summary(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(batch_id): Path<String>,
) -> Response {
    if let Err(resp) = app.guard_full(&headers).await {
        return resp;
    }
    let batch_id = match parse_path_i32(&batch_id, "batch_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match load_match_request_summary(app.pool(), batch_id).await {
        Ok(Some(summary)) => ok_json(summary),
        Ok(None) => err_text(404, "Match request batch not found"),
        Err(err) => {
            tracing::error!(%err, batch_id, "Scrim-Match-Abfrage-Auswertung fehlgeschlagen");
            err_text(500, "Match request summary failed")
        }
    }
}

pub async fn scrims_release_match_request(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let request_id = match parse_path_i32(&request_id, "request_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let input = match parse_match_request_release(&payload) {
        Ok(input) => input,
        Err(resp) => return resp,
    };

    match release_match_request_slot(
        app.pool(),
        request_id,
        input,
        &session.user_id.to_string(),
        &session.display_name,
    )
    .await
    {
        Ok(request) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match_request.release",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                request_id,
                "AUDIT scrims"
            );
            ok_json(json!({ "request": request }))
        }
        Err(MatchRequestReleaseError::BadRequest(message)) => err_text(400, message),
        Err(MatchRequestReleaseError::Conflict(message)) => err_text(409, message),
        Err(MatchRequestReleaseError::NotFound) => err_text(404, "Match request not found"),
        Err(MatchRequestReleaseError::Dashboard(err)) => {
            tracing::error!(%err, request_id, "Scrim-Match-Abfrage-Freigabe-Auswertung fehlgeschlagen");
            err_text(500, "Release match request failed")
        }
        Err(MatchRequestReleaseError::Db(err)) => {
            tracing::error!(%err, request_id, "Scrim-Match-Abfrage-Freigabe fehlgeschlagen");
            err_text(500, "Release match request failed")
        }
    }
}

pub async fn scrims_create_match_request_reminder(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let request_id = match parse_path_i32(&request_id, "request_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let team_id = match parse_i32(get2(&payload, "team_id", "teamId"), "team_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let template = match payload.get("template") {
        None => MATCH_REQUEST_REMINDER_DEFAULT_TEMPLATE,
        Some(Value::String(value))
            if MATCH_REQUEST_REMINDER_TEMPLATES.contains(&value.as_str()) =>
        {
            value
        }
        Some(Value::String(_)) => return err_text(400, "Invalid reminder template"),
        Some(_) => return err_text(400, "template must be string"),
    };

    match create_match_request_reminder_record(
        app.pool(),
        request_id,
        team_id,
        template,
        &session.user_id.to_string(),
        &session.display_name,
    )
    .await
    {
        Ok(Some(reminder)) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match_request.reminder.approve",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                request_id,
                team_id,
                reminder_id = reminder["id"].as_i64().unwrap_or_default(),
                "AUDIT scrims"
            );
            ok_json(json!({ "reminder": reminder }))
        }
        Ok(None) => err_text(404, "Match request team not found"),
        Err(MatchRequestReminderCreateError::BadRequest(message)) => err_text(400, message),
        Err(MatchRequestReminderCreateError::Db(err)) => {
            tracing::error!(%err, request_id, team_id, "Scrim-Reminder-Freigabe fehlgeschlagen");
            err_text(500, "Create reminder failed")
        }
    }
}

pub async fn scrims_start_match(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(_match_id): Path<String>,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, true).await {
        return resp;
    }
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

pub async fn scrims_add_match_id(
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
    let steam_match_id = match parse_match_result_id(&payload) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match add_match_result_ref(
        app.pool(),
        match_id,
        steam_match_id,
        &session.user_id.to_string(),
        &session.display_name,
    )
    .await
    {
        Ok(MatchResultRefUpdate::Updated(scrim_match)) => {
            tracing::info!(
                target: "audit",
                action = "scrim.match.match_id",
                user_id = session.user_id,
                display_name = %session.display_name,
                access = session.access_level.as_str(),
                match_id,
                steam_match_id,
                "AUDIT scrims"
            );
            ok_json(json!({ "match": scrim_match }))
        }
        Ok(MatchResultRefUpdate::Duplicate) => err_text(409, "Match ID already exists"),
        Ok(MatchResultRefUpdate::NotFound) => err_text(404, "Match not found"),
        Err(err) => {
            tracing::error!(%err, match_id, steam_match_id, "Scrim-Match-ID-Speicherung fehlgeschlagen");
            err_text(500, "Save match id failed")
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
    if require_local_scrim_write(
        app.pool(),
        "dl-dashboard::scrims",
        "scrim.participants notes update",
    )
    .await
    .is_err()
    {
        return err_text(503, SCRIM_RUNTIME_DENIED_MESSAGE);
    }
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

pub async fn scrims_create_lagebild_correction(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Path(team_id): Path<String>,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let team_id = match parse_path_i32(&team_id, "team_id") {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return err_text(400, "Invalid JSON body"),
    };
    let message =
        match parse_required_text(get2(&payload, "message", "correction"), "message", 4000) {
            Ok(message) => message,
            Err(resp) => return resp,
        };

    let detail = match load_lagebild_detail(app.pool(), team_id).await {
        Ok(Some(detail)) => detail,
        Ok(None) => return err_text(404, "Scrim team not found"),
        Err(err) => {
            tracing::error!(%err, team_id, "Scrim-Lagebild konnte nicht geladen werden");
            return err_text(500, "Lagebild unavailable");
        }
    };
    let current_snapshot_id = detail
        .get("current")
        .and_then(|current| current.get("id"))
        .and_then(Value::as_i64);
    let current_text = detail
        .get("current")
        .and_then(|current| current.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let team_name = detail
        .get("team_name")
        .and_then(Value::as_str)
        .unwrap_or("Scrim-Team")
        .to_string();
    let evidences = evidences_from_current(&detail);
    let previous_corrections = correction_context_from_detail(&detail);

    let user_message = match insert_lagebild_correction(
        app.pool(),
        team_id,
        current_snapshot_id,
        "user",
        Some(&session.user_id.to_string()),
        Some(&session.display_name),
        &message,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(%err, team_id, "Scrim-Lagebild-Korrektur konnte nicht gespeichert werden");
            return err_text(500, "Save correction failed");
        }
    };

    let Some(provider) = app.ai_provider() else {
        let mut tx = match app.pool().begin().await {
            Ok(tx) => tx,
            Err(err) => {
                tracing::error!(%err, team_id, "Scrim-Lagebild-AI-Fehlertransaktion konnte nicht gestartet werden");
                return err_text(500, "Save correction failed");
            }
        };
        let assistant_message = match insert_lagebild_correction(
            &mut *tx,
            team_id,
            current_snapshot_id,
            "assistant",
            None,
            Some("AI"),
            "Korrektur ist gespeichert. Eine automatische Überarbeitung ist gerade nicht möglich.",
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                tracing::error!(%err, team_id, "Scrim-Lagebild-AI-Fehlerhinweis konnte nicht gespeichert werden");
                return err_text(500, "Save correction failed");
            }
        };
        if let Err(err) = log_scrim_lagebild_decision(
            &mut *tx,
            "scrim.lagebild.correction",
            &format!(
                "team={team_id} current_snapshot={:?} message_chars={}",
                current_snapshot_id,
                message.chars().count()
            ),
            "error",
            "ai_provider_missing",
            "correction_saved",
            json!({
                "team_id": team_id,
                "current_snapshot_id": current_snapshot_id,
                "user_correction_id": user_message["id"],
            }),
        )
        .await
        {
            tracing::error!(%err, team_id, "Scrim-Lagebild-AI-Entscheidung konnte nicht geloggt werden");
            return err_text(500, "AI decision log failed");
        }
        if let Err(err) = tx.commit().await {
            tracing::error!(%err, team_id, "Scrim-Lagebild-AI-Fehlertransaktion konnte nicht gespeichert werden");
            return err_text(500, "Save correction failed");
        }
        return load_lagebild_correction_response(
            app.pool(),
            team_id,
            user_message,
            assistant_message,
        )
        .await;
    };

    match revise_lagebild(
        provider.as_ref(),
        &team_name,
        current_text.as_deref(),
        &message,
        &previous_corrections,
        &evidences,
    )
    .await
    {
        Ok(result) => {
            let mut tx = match app.pool().begin().await {
                Ok(tx) => tx,
                Err(err) => {
                    tracing::error!(%err, team_id, "Scrim-Lagebild-Korrekturtransaktion konnte nicht gestartet werden");
                    return err_text(500, "Save correction failed");
                }
            };
            let assistant_message = match insert_lagebild_correction(
                &mut *tx,
                team_id,
                current_snapshot_id,
                "assistant",
                None,
                Some("AI"),
                &result.reply,
            )
            .await
            {
                Ok(value) => value,
                Err(err) => {
                    tracing::error!(%err, team_id, "Scrim-Lagebild-AI-Antwort konnte nicht gespeichert werden");
                    return err_text(500, "Save correction failed");
                }
            };
            let mut snapshot_id = None;
            let decision;
            let reason;
            let action;
            if let Some(lagebild) = result.lagebild.as_deref() {
                match insert_corrected_lagebild_snapshot(
                    &mut tx,
                    team_id,
                    current_snapshot_id,
                    lagebild,
                    result.model.clone(),
                    assistant_message["id"].as_i64(),
                )
                .await
                {
                    Ok(id) => {
                        snapshot_id = Some(id);
                        decision = "yes";
                        reason = "lagebild_korrigiert";
                        action = "snapshot_created";
                    }
                    Err(err) => {
                        tracing::error!(%err, team_id, "Korrigiertes Scrim-Lagebild konnte nicht gespeichert werden");
                        return err_text(500, "Save lagebild failed");
                    }
                }
            } else {
                decision = "no";
                reason = "erklaerung_ohne_lagebild";
                action = "correction_saved";
            }
            if let Err(err) = log_scrim_lagebild_decision(
                &mut *tx,
                "scrim.lagebild.correction",
                &format!(
                    "team={team_id} current_snapshot={:?} message_chars={}",
                    current_snapshot_id,
                    message.chars().count()
                ),
                decision,
                reason,
                action,
                json!({
                    "team_id": team_id,
                    "current_snapshot_id": current_snapshot_id,
                    "new_snapshot_id": snapshot_id,
                    "user_correction_id": user_message["id"],
                    "assistant_correction_id": assistant_message["id"],
                    "model": result.model,
                }),
            )
            .await
            {
                tracing::error!(%err, team_id, "Scrim-Lagebild-AI-Entscheidung konnte nicht geloggt werden");
                return err_text(500, "AI decision log failed");
            }
            if let Err(err) = tx.commit().await {
                tracing::error!(%err, team_id, "Scrim-Lagebild-Korrekturtransaktion konnte nicht gespeichert werden");
                return err_text(500, "Save correction failed");
            }
            load_lagebild_correction_response(app.pool(), team_id, user_message, assistant_message)
                .await
        }
        Err(err) => {
            tracing::error!(%err, team_id, "Scrim-Lagebild-Korrektur-AI fehlgeschlagen");
            let (decision, reason) = lagebild_correction_failure_decision(&err);
            let mut tx = match app.pool().begin().await {
                Ok(tx) => tx,
                Err(db_err) => {
                    tracing::error!(%db_err, team_id, "Scrim-Lagebild-AI-Fehlertransaktion konnte nicht gestartet werden");
                    return err_text(500, "Save correction failed");
                }
            };
            let assistant_message = match insert_lagebild_correction(
                &mut *tx,
                team_id,
                current_snapshot_id,
                "assistant",
                None,
                Some("AI"),
                "Korrektur ist gespeichert. Die automatische Überarbeitung ist fehlgeschlagen.",
            )
            .await
            {
                Ok(value) => value,
                Err(db_err) => {
                    tracing::error!(%db_err, team_id, "Scrim-Lagebild-AI-Fehlerantwort konnte nicht gespeichert werden");
                    return err_text(500, "Save correction failed");
                }
            };
            if let Err(db_err) = log_scrim_lagebild_decision(
                &mut *tx,
                "scrim.lagebild.correction",
                &format!(
                    "team={team_id} current_snapshot={:?} message_chars={}",
                    current_snapshot_id,
                    message.chars().count()
                ),
                decision,
                reason,
                "correction_saved",
                json!({
                    "team_id": team_id,
                    "current_snapshot_id": current_snapshot_id,
                    "user_correction_id": user_message["id"],
                    "assistant_correction_id": assistant_message["id"],
                    "error": err.to_string(),
                }),
            )
            .await
            {
                tracing::error!(%db_err, team_id, "Scrim-Lagebild-AI-Entscheidung konnte nicht geloggt werden");
                return err_text(500, "AI decision log failed");
            }
            if let Err(db_err) = tx.commit().await {
                tracing::error!(%db_err, team_id, "Scrim-Lagebild-AI-Fehlertransaktion konnte nicht gespeichert werden");
                return err_text(500, "Save correction failed");
            }
            load_lagebild_correction_response(app.pool(), team_id, user_message, assistant_message)
                .await
        }
    }
}

fn lagebild_correction_failure_decision(error: &LagebildError) -> (&'static str, &'static str) {
    match error {
        LagebildError::Provider(ChatProviderError::Timeout) => ("timeout", "ai_timeout"),
        LagebildError::InvalidAi(_) => ("unsure", "ai_response_invalid"),
        _ => ("error", "ai_revision_failed"),
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

struct MatchRequestSummarySeed {
    request_id: i32,
    team_a_id: i32,
    team_a_name: String,
    team_b_id: Option<i32>,
    team_b_name: Option<String>,
    status: String,
    slot_options: Value,
    released_slot_index: Option<i32>,
    released_slot: Option<Value>,
    released_at: Option<DateTime<Utc>>,
    released_by_user_id: Option<String>,
    released_by_display_name: Option<String>,
    override_reason: Option<String>,
}

#[derive(Clone)]
struct MatchRequestMember {
    participant_id: i32,
    display_name: String,
    is_bench: bool,
}

#[derive(Clone)]
struct MatchRequestResponse {
    participant_id: i32,
    slot_index: i32,
    response: String,
}

struct MatchRequestInput {
    team_a_id: i32,
    team_b_id: Option<i32>,
    slots: Vec<Value>,
}

struct MatchRequestReleaseInput {
    slot_index: Option<i32>,
    reason: Option<String>,
}

struct SlotPresetInput {
    name: String,
    slots: Vec<Value>,
}

#[derive(Clone)]
struct ScrimTeamOption {
    id: i64,
    name: String,
}

#[derive(Debug, thiserror::Error)]
enum MatchRequestCreateError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, thiserror::Error)]
enum MatchRequestReleaseError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Dashboard(#[from] DashboardDbError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, thiserror::Error)]
enum MatchRequestReminderCreateError {
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

fn match_request_defaults_json(presets: Vec<Value>) -> Value {
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
        "presets": presets
    })
}

fn default_match_request_slots() -> Vec<Value> {
    vec![
        json!({ "day": "sat", "from": 20 * 60, "to": 22 * 60 }),
        json!({ "day": "sun", "from": 20 * 60, "to": 22 * 60 }),
    ]
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

fn parse_match_request_release(payload: &Value) -> Result<MatchRequestReleaseInput, Response> {
    let slot_index = parse_optional_i32(get2(payload, "slot_index", "slotIndex"), "slot_index")?;
    if slot_index.is_some_and(|index| index < 0) {
        return Err(err_text(400, "slot_index must be non-negative"));
    }
    Ok(MatchRequestReleaseInput {
        slot_index,
        reason: parse_optional_text(get2(payload, "reason", "overrideReason"), "reason", 1000)?,
    })
}

fn parse_slot_preset_body(body: &[u8]) -> Result<SlotPresetInput, Response> {
    let payload = match serde_json::from_slice::<Value>(body) {
        Ok(value) if value.is_object() => value,
        _ => return Err(err_text(400, "Invalid JSON body")),
    };
    Ok(SlotPresetInput {
        name: parse_required_text(payload.get("name"), "name", 100)?,
        slots: parse_match_request_slots(
            payload
                .get("slots")
                .ok_or_else(|| err_text(400, "slots must be an array"))?,
        )?,
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

fn parse_path_i64(raw: &str, field: &str) -> Result<i64, Response> {
    raw.trim()
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| err_text(400, &format!("{field} must be a positive integer")))
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
    parse_optional_text(
        payload.get("notes").or_else(|| payload.get("note")),
        "notes",
        4000,
    )
}

fn parse_optional_text(
    raw: Option<&Value>,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, Response> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => {
            if s.chars().count() > max_chars {
                return Err(err_text(
                    400,
                    &format!("{field} must be at most {max_chars} characters"),
                ));
            }
            Ok(Some(s.trim().to_string()))
        }
        Some(_) => Err(err_text(400, &format!("{field} must be string"))),
    }
}

fn parse_required_text(
    raw: Option<&Value>,
    field: &str,
    max_chars: usize,
) -> Result<String, Response> {
    match parse_optional_text(raw, field, max_chars)? {
        Some(value) => Ok(value),
        None => Err(err_text(400, &format!("{field} must be string"))),
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

fn parse_match_result_id(payload: &Value) -> Result<i64, Response> {
    let raw = get2(payload, "match_id", "matchId")
        .or_else(|| get2(payload, "steam_match_id", "steamMatchId"))
        .or_else(|| get2(payload, "deadlock_match_id", "deadlockMatchId"));
    let value = match raw {
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| err_text(400, "match_id must be a positive integer"))?,
        Some(Value::String(value)) => value
            .trim()
            .parse::<i64>()
            .map_err(|_| err_text(400, "match_id must be a positive integer"))?,
        _ => return Err(err_text(400, "match_id must be a positive integer")),
    };
    if value <= 0 {
        return Err(err_text(400, "match_id must be a positive integer"));
    }
    Ok(value)
}

async fn load_overview(pool: &PgPool) -> DashboardDbResult<Value> {
    let participants = load_participants(pool).await?;
    let teams = load_teams(pool).await?;
    let matches = load_matches(pool).await?;
    let match_request_summaries = load_recent_match_request_summaries(pool).await?;
    let suggested_block = load_suggested_block(pool, &teams).await?;
    let lagebilder = load_lagebilder(pool).await?;
    Ok(json!({
        "teams": teams,
        "participants": participants,
        "matches": matches,
        "match_request_summaries": match_request_summaries,
        "suggested_block": suggested_block,
        "lagebilder": lagebilder,
    }))
}

async fn load_slot_presets(pool: &PgPool) -> DashboardDbResult<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, slots, created_by_user_id, created_at, updated_at
          FROM scrim.slot_presets
         ORDER BY created_at ASC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(slot_preset_json).collect()
}

fn slot_preset_json(row: sqlx::postgres::PgRow) -> DashboardDbResult<Value> {
    let id = row.try_get::<i64, _>("id")?;
    Ok(json!({
        "id": id,
        // ponytail: "key" liest niemand mehr, das Frontend adressiert Presets über "id".
        // Bleibt nur, weil der Defaults-Vertrag es bisher zusagt; beim naechsten Anfassen loeschen.
        "key": if id == 1 { "weekend_evening".to_string() } else { format!("slot_preset_{id}") },
        "name": row.try_get::<String, _>("name")?,
        "slots": row.try_get::<Value, _>("slots")?,
        "created_by_user_id": row.try_get::<String, _>("created_by_user_id")?,
        "created_at": row.try_get::<DateTime<Utc>, _>("created_at")?.timestamp(),
        "updated_at": row.try_get::<DateTime<Utc>, _>("updated_at")?.timestamp(),
    }))
}

async fn create_slot_preset(
    pool: &PgPool,
    created_by_user_id: &str,
    input: SlotPresetInput,
) -> DashboardDbResult<Value> {
    let row = sqlx::query(
        r#"
        INSERT INTO scrim.slot_presets(name, slots, created_by_user_id)
        VALUES($1, $2::jsonb, $3)
        RETURNING id, name, slots, created_by_user_id, created_at, updated_at
        "#,
    )
    .bind(input.name)
    .bind(Value::Array(input.slots))
    .bind(created_by_user_id)
    .fetch_one(pool)
    .await?;
    slot_preset_json(row)
}

async fn update_slot_preset(
    pool: &PgPool,
    preset_id: i64,
    input: SlotPresetInput,
) -> DashboardDbResult<Option<Value>> {
    sqlx::query(
        r#"
        UPDATE scrim.slot_presets
           SET name = $2,
               slots = $3::jsonb,
               updated_at = now()
         WHERE id = $1
        RETURNING id, name, slots, created_by_user_id, created_at, updated_at
        "#,
    )
    .bind(preset_id)
    .bind(input.name)
    .bind(Value::Array(input.slots))
    .fetch_optional(pool)
    .await?
    .map(slot_preset_json)
    .transpose()
}

async fn delete_slot_preset(pool: &PgPool, preset_id: i64) -> DashboardDbResult<bool> {
    Ok(sqlx::query("DELETE FROM scrim.slot_presets WHERE id = $1")
        .bind(preset_id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

async fn load_lagebilder(pool: &PgPool) -> DashboardDbResult<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT id::bigint AS team_id, name AS team_name
          FROM scrim.teams
         ORDER BY name ASC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut items = rows
        .into_iter()
        .map(|row| {
            Ok(json!({
                "team_id": row.try_get::<i64, _>("team_id")?,
                "team_name": row.try_get::<String, _>("team_name")?,
            }))
        })
        .collect::<DashboardDbResult<Vec<_>>>()?;
    attach_lagebild_data(pool, &mut items).await?;
    Ok(items)
}

async fn load_lagebild_detail(pool: &PgPool, team_id: i32) -> DashboardDbResult<Option<Value>> {
    let Some(row) = sqlx::query(
        r#"
        SELECT id::bigint AS team_id, name AS team_name
          FROM scrim.teams
         WHERE id = $1
        "#,
    )
    .bind(team_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let mut items = vec![json!({
        "team_id": row.try_get::<i64, _>("team_id")?,
        "team_name": row.try_get::<String, _>("team_name")?,
    })];
    attach_lagebild_data(pool, &mut items).await?;
    Ok(items.pop())
}

async fn attach_lagebild_data(pool: &PgPool, items: &mut [Value]) -> DashboardDbResult<()> {
    let team_ids = items
        .iter()
        .filter_map(|item| item.get("team_id").and_then(Value::as_i64))
        .map(|id| i64_to_i32(id, "team_id"))
        .collect::<DashboardDbResult<Vec<_>>>()?;
    if team_ids.is_empty() {
        return Ok(());
    }
    let mut snapshots_by_team = load_lagebild_snapshots(pool, &team_ids).await?;
    let mut corrections_by_team = load_lagebild_corrections(pool, &team_ids).await?;

    for item in items {
        let Some(team_id) = item.get("team_id").and_then(Value::as_i64) else {
            continue;
        };
        let timeline = snapshots_by_team.remove(&team_id).unwrap_or_default();
        let current = timeline.first().cloned().unwrap_or(Value::Null);
        if let Some(obj) = item.as_object_mut() {
            obj.insert("current".to_string(), current);
            obj.insert("timeline".to_string(), Value::Array(timeline));
            obj.insert(
                "corrections".to_string(),
                Value::Array(corrections_by_team.remove(&team_id).unwrap_or_default()),
            );
        }
    }
    Ok(())
}

async fn load_lagebild_snapshots(
    pool: &PgPool,
    team_ids: &[i32],
) -> DashboardDbResult<HashMap<i64, Vec<Value>>> {
    let rows = sqlx::query(
        r#"
        SELECT id,
               team_id::bigint AS team_id,
               generated_at,
               generated_for,
               source,
               status,
               lagebild_text,
               data_summary,
               model,
               error,
               created_at
          FROM scrim.lagebild_snapshots
         WHERE team_id = ANY($1)
         ORDER BY team_id ASC, generated_at DESC, id DESC
        "#,
    )
    .bind(team_ids)
    .fetch_all(pool)
    .await?;
    let mut snapshots = Vec::with_capacity(rows.len());
    let mut snapshot_ids = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row.try_get::<i64, _>("id")?;
        snapshot_ids.push(id);
        snapshots.push(json!({
            "id": id,
            "team_id": row.try_get::<i64, _>("team_id")?,
            "generated_at": utc_to_json_unix(Some(row.try_get::<DateTime<Utc>, _>("generated_at")?)),
            "generated_for": row.try_get::<String, _>("generated_for")?,
            "source": row.try_get::<String, _>("source")?,
            "status": row.try_get::<String, _>("status")?,
            "text": row.try_get::<String, _>("lagebild_text")?,
            "data_summary": row.try_get::<Value, _>("data_summary")?,
            "model": row.try_get::<Option<String>, _>("model")?,
            "error": row.try_get::<Option<String>, _>("error")?,
            "created_at": utc_to_json_unix(Some(row.try_get::<DateTime<Utc>, _>("created_at")?)),
        }));
    }
    let mut evidences_by_snapshot = load_lagebild_evidences(pool, &snapshot_ids).await?;
    let mut by_team: HashMap<i64, Vec<Value>> = HashMap::new();
    for mut snapshot in snapshots {
        let snapshot_id = snapshot
            .get("id")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let team_id = snapshot
            .get("team_id")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert(
                "evidences".to_string(),
                Value::Array(
                    evidences_by_snapshot
                        .remove(&snapshot_id)
                        .unwrap_or_default(),
                ),
            );
        }
        by_team.entry(team_id).or_default().push(snapshot);
    }
    Ok(by_team)
}

async fn load_lagebild_evidences(
    pool: &PgPool,
    snapshot_ids: &[i64],
) -> DashboardDbResult<HashMap<i64, Vec<Value>>> {
    if snapshot_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT id,
               snapshot_id,
               evidence_type,
               label,
               url,
               reference_id,
               occurred_at,
               payload
          FROM scrim.lagebild_evidences
         WHERE snapshot_id = ANY($1)
         ORDER BY snapshot_id ASC, occurred_at DESC NULLS LAST, id ASC
        "#,
    )
    .bind(snapshot_ids)
    .fetch_all(pool)
    .await?;
    let mut by_snapshot: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows {
        let snapshot_id = row.try_get::<i64, _>("snapshot_id")?;
        by_snapshot
            .entry(snapshot_id)
            .or_default()
            .push(lagebild_evidence_json(row)?);
    }
    Ok(by_snapshot)
}

fn lagebild_evidence_json(row: sqlx::postgres::PgRow) -> DashboardDbResult<Value> {
    let url = row
        .try_get::<Option<String>, _>("url")?
        .filter(|value| is_allowed_lagebild_evidence_url(value));
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "snapshot_id": row.try_get::<i64, _>("snapshot_id")?,
        "type": row.try_get::<String, _>("evidence_type")?,
        "label": row.try_get::<String, _>("label")?,
        "url": url,
        "reference_id": row.try_get::<Option<String>, _>("reference_id")?,
        "occurred_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("occurred_at")?),
        "payload": row.try_get::<Value, _>("payload")?,
    }))
}

fn is_allowed_lagebild_evidence_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("discord.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some(segments) = url.path_segments() else {
        return false;
    };
    let segments = segments.collect::<Vec<_>>();
    segments.len() == 4
        && segments[0] == "channels"
        && segments[1] == SCRIM_MAIN_GUILD_ID.to_string()
        && segments[2].parse::<u64>().is_ok_and(|id| id > 0)
        && segments[3].parse::<u64>().is_ok_and(|id| id > 0)
}

async fn load_lagebild_corrections(
    pool: &PgPool,
    team_ids: &[i32],
) -> DashboardDbResult<HashMap<i64, Vec<Value>>> {
    let rows = sqlx::query(
        r#"
        SELECT id,
               team_id::bigint AS team_id,
               snapshot_id,
               role,
               author_user_id,
               author_display_name,
               message,
               created_at
          FROM scrim.lagebild_corrections
         WHERE team_id = ANY($1)
         ORDER BY team_id ASC, created_at ASC, id ASC
        "#,
    )
    .bind(team_ids)
    .fetch_all(pool)
    .await?;
    let mut by_team: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows {
        let team_id = row.try_get::<i64, _>("team_id")?;
        by_team
            .entry(team_id)
            .or_default()
            .push(lagebild_correction_json(row)?);
    }
    Ok(by_team)
}

fn lagebild_correction_json(row: sqlx::postgres::PgRow) -> DashboardDbResult<Value> {
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "team_id": row.try_get::<i64, _>("team_id")?,
        "snapshot_id": row.try_get::<Option<i64>, _>("snapshot_id")?,
        "role": row.try_get::<String, _>("role")?,
        "author_user_id": row.try_get::<Option<String>, _>("author_user_id")?,
        "author_display_name": row.try_get::<Option<String>, _>("author_display_name")?,
        "message": row.try_get::<String, _>("message")?,
        "created_at": utc_to_json_unix(Some(row.try_get::<DateTime<Utc>, _>("created_at")?)),
    }))
}

async fn load_suggested_block(pool: &PgPool, teams: &[Value]) -> DashboardDbResult<Value> {
    let batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scrim.match_request_batches")
        .fetch_one(pool)
        .await?;
    Ok(suggested_block_json(
        teams,
        batch_count.max(0) as usize,
        Utc::now(),
    ))
}

fn suggested_block_json(teams: &[Value], rotation_seed: usize, now: DateTime<Utc>) -> Value {
    json!({
        "key": "two_week_scrim_block",
        "name": "Zwei-Wochen-Scrim-Block",
        "rhythm_days": 14,
        "template": "regular_scrim",
        "deadline_at": utc_to_json_unix(Some(now + ChronoDuration::hours(MATCH_REQUEST_DEFAULT_DEADLINE_HOURS))),
        "slots": default_match_request_slots(),
        "matches": suggested_pairings(teams, rotation_seed),
    })
}

fn suggested_pairings(teams: &[Value], rotation_seed: usize) -> Vec<Value> {
    let mut teams = teams
        .iter()
        .filter_map(|team| {
            Some(ScrimTeamOption {
                id: team.get("id")?.as_i64()?,
                name: team
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect::<Vec<_>>();
    teams.sort_by_key(|team| team.id);

    let half = teams.len().div_ceil(2);
    let (left, right) = teams.split_at(half);
    let mut right = right.to_vec();
    if !right.is_empty() {
        let shift = rotation_seed % right.len();
        right.rotate_left(shift);
    }

    left.iter()
        .zip(right.iter())
        .map(|(team_a, team_b)| {
            json!({
                "team_a_id": team_a.id,
                "team_a_name": team_a.name,
                "team_b_id": team_b.id,
                "team_b_name": team_b.name,
            })
        })
        .collect()
}

async fn load_recent_match_request_summaries(pool: &PgPool) -> DashboardDbResult<Vec<Value>> {
    let batch_ids = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id
          FROM scrim.match_request_batches
         WHERE status IN ('draft', 'posting', 'open', 'post_failed', 'closed')
         ORDER BY deadline_at DESC, id DESC
         LIMIT $1
        "#,
    )
    .bind(MATCH_REQUEST_SUMMARY_LIMIT)
    .fetch_all(pool)
    .await?;
    let mut summaries = Vec::with_capacity(batch_ids.len());
    for batch_id in batch_ids {
        if let Some(summary) = load_match_request_summary(pool, batch_id).await? {
            summaries.push(summary);
        }
    }
    Ok(summaries)
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
    let mut matches = rows
        .into_iter()
        .map(match_json)
        .collect::<DashboardDbResult<Vec<_>>>()?;
    attach_match_result_refs(pool, &mut matches).await?;
    Ok(matches)
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

async fn attach_match_result_refs(pool: &PgPool, matches: &mut [Value]) -> DashboardDbResult<()> {
    let ids = matches
        .iter()
        .filter_map(|scrim_match| scrim_match.get("id").and_then(Value::as_i64))
        .map(|id| i64_to_i32(id, "match_id"))
        .collect::<DashboardDbResult<Vec<_>>>()?;
    if ids.is_empty() {
        return Ok(());
    }

    let rows = sqlx::query(
        r#"
        SELECT id::bigint AS id,
               match_id::bigint AS match_id,
               steam_match_id,
               source_user_id,
               source_display_name,
               entered_at,
               fetch_status,
               fetched_at,
               last_error,
               winner_team_id::bigint AS winner_team_id,
               raw_result_json,
               normalized_result_json,
               updated_at
          FROM scrim.match_result_refs
         WHERE match_id = ANY($1)
         ORDER BY entered_at ASC, id ASC
        "#,
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    let mut refs_by_match: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows {
        let match_id = row.try_get::<i64, _>("match_id")?;
        refs_by_match
            .entry(match_id)
            .or_default()
            .push(match_result_ref_json(row)?);
    }

    for scrim_match in matches {
        let Some(match_id) = scrim_match.get("id").and_then(Value::as_i64) else {
            continue;
        };
        if let Some(obj) = scrim_match.as_object_mut() {
            obj.insert(
                "match_result_refs".to_string(),
                Value::Array(refs_by_match.remove(&match_id).unwrap_or_default()),
            );
        }
    }
    Ok(())
}

fn match_result_ref_json(row: sqlx::postgres::PgRow) -> DashboardDbResult<Value> {
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "match_id": row.try_get::<i64, _>("match_id")?,
        "steam_match_id": row.try_get::<i64, _>("steam_match_id")?,
        "source_user_id": row.try_get::<String, _>("source_user_id")?,
        "source_display_name": row.try_get::<String, _>("source_display_name")?,
        "entered_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("entered_at")?),
        "fetch_status": row.try_get::<String, _>("fetch_status")?,
        "fetched_at": utc_to_json_unix(row.try_get::<Option<DateTime<Utc>>, _>("fetched_at")?),
        "last_error": row.try_get::<Option<String>, _>("last_error")?,
        "winner_team_id": row.try_get::<Option<i64>, _>("winner_team_id")?,
        "raw_result_json": row.try_get::<Option<Value>, _>("raw_result_json")?,
        "normalized_result_json": row.try_get::<Option<Value>, _>("normalized_result_json")?.unwrap_or_else(|| json!({})),
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

async fn insert_lagebild_correction<'e, E>(
    executor: E,
    team_id: i32,
    snapshot_id: Option<i64>,
    role: &str,
    author_user_id: Option<&str>,
    author_display_name: Option<&str>,
    message: &str,
) -> DashboardDbResult<Value>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"
        INSERT INTO scrim.lagebild_corrections(
            team_id, snapshot_id, role, author_user_id, author_display_name, message, created_at
        )
        VALUES($1, $2, $3, $4, $5, $6, now())
        RETURNING id, team_id::bigint AS team_id, snapshot_id, role,
                  author_user_id, author_display_name, message, created_at
        "#,
    )
    .bind(team_id)
    .bind(snapshot_id)
    .bind(role)
    .bind(author_user_id)
    .bind(author_display_name)
    .bind(message)
    .fetch_one(executor)
    .await?;
    lagebild_correction_json(row)
}

async fn insert_corrected_lagebild_snapshot(
    connection: &mut PgConnection,
    team_id: i32,
    previous_snapshot_id: Option<i64>,
    text: &str,
    model: Option<String>,
    correction_id: Option<i64>,
) -> DashboardDbResult<i64> {
    let generated_for = correction_id
        .map(|id| format!("correction:{id}"))
        .unwrap_or_else(|| "correction".to_string());
    let snapshot_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO scrim.lagebild_snapshots(
            team_id, generated_for, source, status, lagebild_text, data_summary, model,
            generated_at, created_at
        )
        VALUES($1, $2, 'correction', 'ok', $3, $4::jsonb, $5, now(), now())
        RETURNING id
        "#,
    )
    .bind(team_id)
    .bind(&generated_for)
    .bind(text)
    .bind(json!({
        "previous_snapshot_id": previous_snapshot_id,
        "correction_id": correction_id,
    }))
    .bind(model)
    .fetch_one(&mut *connection)
    .await?;
    if let Some(previous_snapshot_id) = previous_snapshot_id {
        sqlx::query(
            r#"
            INSERT INTO scrim.lagebild_evidences(
                snapshot_id, evidence_type, label, url, reference_id, occurred_at, payload
            )
            SELECT $1, evidence_type, label, url, reference_id, occurred_at, payload
              FROM scrim.lagebild_evidences
             WHERE snapshot_id = $2
             ORDER BY id ASC
            "#,
        )
        .bind(snapshot_id)
        .bind(previous_snapshot_id)
        .execute(&mut *connection)
        .await?;
    }
    Ok(snapshot_id)
}

async fn log_scrim_lagebild_decision<'e, E>(
    executor: E,
    source: &str,
    input_summary: &str,
    decision: &str,
    reason: &str,
    action_taken: &str,
    payload: Value,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO bot.ai_decision_ledger(
            source, subject_user_id, guild_id, input_summary, decision,
            confidence, reason, action_taken, payload
        )
        VALUES($1, NULL, $2, $3, $4, NULL, $5, $6, $7::jsonb)
        "#,
    )
    .bind(source)
    .bind(i64::try_from(SCRIM_MAIN_GUILD_ID).ok())
    .bind(input_summary)
    .bind(decision)
    .bind(reason)
    .bind(action_taken)
    .bind(payload)
    .execute(executor)
    .await?;
    Ok(())
}

async fn load_lagebild_correction_response(
    pool: &PgPool,
    team_id: i32,
    user_message: Value,
    assistant_message: Value,
) -> Response {
    match load_lagebild_detail(pool, team_id).await {
        Ok(Some(detail)) => {
            let current = detail.get("current").cloned().unwrap_or(Value::Null);
            let timeline = detail.get("timeline").cloned().unwrap_or_else(|| json!([]));
            let corrections = detail
                .get("corrections")
                .cloned()
                .unwrap_or_else(|| json!([]));
            ok_json(json!({
                "team": detail,
                "current": current,
                "timeline": timeline,
                "corrections": corrections,
                "user_message": user_message,
                "assistant_message": assistant_message,
            }))
        }
        Ok(None) => err_text(404, "Scrim team not found"),
        Err(err) => {
            tracing::error!(%err, team_id, "Scrim-Lagebild nach Korrektur konnte nicht geladen werden");
            err_text(500, "Lagebild unavailable")
        }
    }
}

fn evidences_from_current(detail: &Value) -> Vec<ScrimLagebildEvidence> {
    detail
        .get("current")
        .and_then(|current| current.get("evidences"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|evidence| {
            let label = evidence.get("label")?.as_str()?.to_string();
            let occurred_at = evidence
                .get("occurred_at")
                .and_then(Value::as_i64)
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
                .map(|dt| dt.to_rfc3339());
            Some(ScrimLagebildEvidence {
                evidence_type: evidence
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("reference")
                    .to_string(),
                label,
                url: evidence
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                reference_id: evidence
                    .get("reference_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                occurred_at,
                payload: evidence
                    .get("payload")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            })
        })
        .collect()
}

fn correction_context_from_detail(detail: &Value) -> Vec<String> {
    detail
        .get("corrections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|correction| {
            let role = correction.get("role")?.as_str()?;
            let message = correction.get("message")?.as_str()?;
            Some(format!("{role}: {message}"))
        })
        .collect()
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

async fn release_match_request_slot(
    pool: &PgPool,
    request_id: i32,
    input: MatchRequestReleaseInput,
    released_by_user_id: &str,
    released_by_display_name: &str,
) -> Result<Value, MatchRequestReleaseError> {
    let mut tx = pool.begin().await?;
    let Some(row) = sqlx::query(
        r#"
        SELECT mr.batch_id, mr.slot_options, mr.status, b.deadline_at
          FROM scrim.match_requests mr
          JOIN scrim.match_request_batches b ON b.id = mr.batch_id
         WHERE mr.id = $1
         FOR UPDATE OF mr
        "#,
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Err(MatchRequestReleaseError::NotFound);
    };

    let status = row.try_get::<String, _>("status")?;
    if !matches!(status.as_str(), "open" | "post_failed") {
        return Err(MatchRequestReleaseError::Conflict(
            "Match request is not open",
        ));
    }

    let batch_id = row.try_get::<i32, _>("batch_id")?;
    let deadline_at = row.try_get::<DateTime<Utc>, _>("deadline_at")?;
    if deadline_at > Utc::now() {
        return Err(MatchRequestReleaseError::Conflict(
            "Deadline has not passed",
        ));
    }

    let slot_options = row.try_get::<Value, _>("slot_options")?;
    let slots = slot_options
        .as_array()
        .ok_or(MatchRequestReleaseError::BadRequest(
            "Stored slots are invalid",
        ))?;
    let summary = load_match_request_summary(pool, batch_id)
        .await?
        .ok_or(MatchRequestReleaseError::NotFound)?;
    let request_summary = match_request_from_summary(&summary, request_id)
        .ok_or(MatchRequestReleaseError::NotFound)?;
    let recommended_slot_index = request_summary["recommended_slot_index"]
        .as_i64()
        .and_then(|value| i32::try_from(value).ok());
    let released_slot_index =
        input
            .slot_index
            .or(recommended_slot_index)
            .ok_or(MatchRequestReleaseError::Conflict(
                "No recommended slot available",
            ))?;
    let released_slot = slots
        .get(
            usize::try_from(released_slot_index)
                .map_err(|_| MatchRequestReleaseError::BadRequest("slot_index is out of range"))?,
        )
        .cloned()
        .ok_or(MatchRequestReleaseError::BadRequest(
            "slot_index is out of range",
        ))?;
    let override_reason =
        if input.slot_index.is_some() && Some(released_slot_index) != recommended_slot_index {
            input.reason
        } else {
            None
        };

    sqlx::query(
        r#"
        UPDATE scrim.match_requests
           SET released_slot_index = $2,
               released_slot = $3::jsonb,
               released_at = now(),
               released_by_user_id = $4,
               released_by_display_name = $5,
               override_reason = $6,
               status_message_state = 'pending',
               status_message_last_error = NULL,
               status = 'closed',
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(request_id)
    .bind(released_slot_index)
    .bind(&released_slot)
    .bind(released_by_user_id)
    .bind(released_by_display_name)
    .bind(&override_reason)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE scrim.match_request_batches b
           SET status = 'closed',
               updated_at = now()
         WHERE b.id = $1
           AND NOT EXISTS (
                SELECT 1
                  FROM scrim.match_requests mr
                 WHERE mr.batch_id = b.id
                   AND mr.status NOT IN ('closed', 'cancelled')
           )
        "#,
    )
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let summary = load_match_request_summary(pool, batch_id)
        .await?
        .ok_or(MatchRequestReleaseError::NotFound)?;
    match_request_from_summary(&summary, request_id).ok_or(MatchRequestReleaseError::NotFound)
}

async fn create_match_request_reminder_record(
    pool: &PgPool,
    request_id: i32,
    team_id: i32,
    template: &str,
    approved_by_user_id: &str,
    approved_by_display_name: &str,
) -> Result<Option<Value>, MatchRequestReminderCreateError> {
    let Some(request) = sqlx::query(
        r#"
        SELECT mr.status,
               mr.team_query_message_ids,
               t.name AS team_name,
               t.discord_channel_id,
               t.discord_role_id
          FROM scrim.match_requests mr
          JOIN scrim.teams t ON t.id = $2
         WHERE mr.id = $1
           AND (mr.team_a_id = $2 OR mr.team_b_id = $2)
        "#,
    )
    .bind(request_id)
    .bind(team_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let status = request.try_get::<String, _>("status")?;
    if status != "open" && status != "post_failed" {
        return Err(MatchRequestReminderCreateError::BadRequest(
            "Match request is not open",
        ));
    }
    let channel_id = request
        .try_get::<Option<i64>, _>("discord_channel_id")?
        .filter(|id| *id > 0)
        .ok_or(MatchRequestReminderCreateError::BadRequest(
            "Team channel missing",
        ))?;
    let message_ids = request.try_get::<Value, _>("team_query_message_ids")?;
    let source =
        message_ids
            .get(team_id.to_string())
            .ok_or(MatchRequestReminderCreateError::BadRequest(
                "Original team query missing",
            ))?;
    let source_channel_id = source
        .get("channel_id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .ok_or(MatchRequestReminderCreateError::BadRequest(
            "Original team query missing",
        ))?;
    if source_channel_id != channel_id {
        return Err(MatchRequestReminderCreateError::BadRequest(
            "Original team query channel mismatch",
        ));
    }
    let source_message_id = source
        .get("message_id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .ok_or(MatchRequestReminderCreateError::BadRequest(
            "Original team query missing",
        ))?;

    let missing_rows = sqlx::query(
        r#"
        SELECT p.id, p.discord_id
          FROM scrim.team_members tm
          JOIN scrim.participants p ON p.id = tm.participant_id
         WHERE tm.team_id = $1
           AND NOT EXISTS (
                SELECT 1
                  FROM scrim.match_request_responses r
                 WHERE r.request_id = $2
                   AND r.team_id = $1
                   AND r.participant_id = p.id
           )
         ORDER BY tm.is_bench ASC, p.display_name ASC, p.id ASC
        "#,
    )
    .bind(team_id)
    .bind(request_id)
    .fetch_all(pool)
    .await?;
    if missing_rows.is_empty() {
        return Err(MatchRequestReminderCreateError::BadRequest(
            "No missing responses",
        ));
    }

    let mut target_participant_ids = Vec::with_capacity(missing_rows.len());
    let mut target_discord_user_ids = Vec::new();
    for row in missing_rows {
        target_participant_ids.push(row.try_get::<i32, _>("id")?);
        if let Some(discord_id) = row.try_get::<Option<i64>, _>("discord_id")? {
            if discord_id > 0 {
                target_discord_user_ids.push(discord_id);
            }
        }
    }
    let target_kind = if target_discord_user_ids.len() == target_participant_ids.len() {
        "members"
    } else {
        "team"
    };
    if target_kind == "team" {
        target_discord_user_ids.clear();
    }
    let target_role_id = if target_kind == "team" {
        request.try_get::<Option<i64>, _>("discord_role_id")?
    } else {
        None
    };
    if target_kind == "team" && target_role_id.is_none() {
        return Err(MatchRequestReminderCreateError::BadRequest(
            "Team role missing for ambiguous reminder",
        ));
    }
    let missing_count = i32::try_from(target_participant_ids.len())
        .map_err(|_| MatchRequestReminderCreateError::BadRequest("Too many missing responses"))?;

    let row = sqlx::query(
        r#"
        INSERT INTO scrim.match_request_reminders(
            action, request_id, team_id, template, target_kind, target_participant_ids,
            target_discord_user_ids, target_role_id, missing_count, approved_by_user_id,
            approved_by_display_name, approved_at, scheduled_for, status, discord_channel_id,
            source_message_id, created_at, updated_at
        )
        VALUES(
            'missing_response_reminder', $1, $2, $3, $4, $5, $6, $7, $8, $9,
            $10, now(), now(), 'approved', $11, $12, now(), now()
        )
        RETURNING id::bigint AS id, request_id, team_id, action, template, target_kind,
                  missing_count, approved_at, scheduled_for, status
        "#,
    )
    .bind(request_id)
    .bind(team_id)
    .bind(template)
    .bind(target_kind)
    .bind(&target_participant_ids)
    .bind(&target_discord_user_ids)
    .bind(target_role_id)
    .bind(missing_count)
    .bind(approved_by_user_id)
    .bind(approved_by_display_name)
    .bind(channel_id)
    .bind(source_message_id)
    .fetch_one(pool)
    .await?;

    Ok(Some(json!({
        "id": row.try_get::<i64, _>("id")?,
        "request_id": row.try_get::<i32, _>("request_id")?,
        "team_id": row.try_get::<i32, _>("team_id")?,
        "action": row.try_get::<String, _>("action")?,
        "template": row.try_get::<String, _>("template")?,
        "target_kind": row.try_get::<String, _>("target_kind")?,
        "missing_count": row.try_get::<i32, _>("missing_count")?,
        "approved_at": utc_to_json_unix(Some(row.try_get::<DateTime<Utc>, _>("approved_at")?)),
        "scheduled_for": utc_to_json_unix(Some(row.try_get::<DateTime<Utc>, _>("scheduled_for")?)),
        "status": row.try_get::<String, _>("status")?,
    })))
}

async fn load_match_request_summary(
    pool: &PgPool,
    batch_id: i32,
) -> DashboardDbResult<Option<Value>> {
    let Some(batch) = sqlx::query(
        r#"
        SELECT id::bigint AS id, template, deadline_at, status
          FROM scrim.match_request_batches
         WHERE id = $1
        "#,
    )
    .bind(batch_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let deadline_at = batch.try_get::<DateTime<Utc>, _>("deadline_at")?;
    let request_rows = sqlx::query(
        r#"
        SELECT mr.id,
               mr.team_a_id,
               ta.name AS team_a_name,
               mr.team_b_id,
               tb.name AS team_b_name,
               mr.status,
               mr.slot_options,
               mr.released_slot_index,
               mr.released_slot,
               mr.released_at,
               mr.released_by_user_id,
               mr.released_by_display_name,
               mr.override_reason
          FROM scrim.match_requests mr
          JOIN scrim.teams ta ON ta.id = mr.team_a_id
          LEFT JOIN scrim.teams tb ON tb.id = mr.team_b_id
         WHERE mr.batch_id = $1
         ORDER BY mr.id ASC
        "#,
    )
    .bind(batch_id)
    .fetch_all(pool)
    .await?;

    let requests = request_rows
        .into_iter()
        .map(|row| {
            Ok(MatchRequestSummarySeed {
                request_id: row.try_get("id")?,
                team_a_id: row.try_get("team_a_id")?,
                team_a_name: row.try_get("team_a_name")?,
                team_b_id: row.try_get("team_b_id")?,
                team_b_name: row.try_get("team_b_name")?,
                status: row.try_get("status")?,
                slot_options: row.try_get("slot_options")?,
                released_slot_index: row.try_get("released_slot_index")?,
                released_slot: row.try_get("released_slot")?,
                released_at: row.try_get("released_at")?,
                released_by_user_id: row.try_get("released_by_user_id")?,
                released_by_display_name: row.try_get("released_by_display_name")?,
                override_reason: row.try_get("override_reason")?,
            })
        })
        .collect::<DashboardDbResult<Vec<_>>>()?;

    let team_ids = requests
        .iter()
        .flat_map(|request| [Some(request.team_a_id), request.team_b_id])
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let request_ids = requests
        .iter()
        .map(|request| request.request_id)
        .collect::<Vec<_>>();
    let members = load_match_request_members(pool, &team_ids).await?;
    let responses = load_match_request_responses(pool, &request_ids).await?;
    let deadline_passed = deadline_at <= Utc::now();
    let mut matches = Vec::with_capacity(requests.len());

    for request in requests {
        matches.push(match_request_summary_json(
            &request,
            &members,
            &responses,
            deadline_passed,
        ));
    }

    Ok(Some(json!({
        "batch": {
            "id": batch.try_get::<i64, _>("id")?,
            "template": batch.try_get::<String, _>("template")?,
            "status": batch.try_get::<String, _>("status")?,
            "deadline_at": utc_to_json_unix(Some(deadline_at)),
        },
        "deadline_passed": deadline_passed,
        "matches": matches,
    })))
}

fn match_request_from_summary(summary: &Value, request_id: i32) -> Option<Value> {
    summary["matches"].as_array()?.iter().find_map(|request| {
        (request["request_id"].as_i64() == Some(i64::from(request_id))).then(|| request.clone())
    })
}

async fn load_match_request_members(
    pool: &PgPool,
    team_ids: &[i32],
) -> DashboardDbResult<HashMap<i32, Vec<MatchRequestMember>>> {
    if team_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT tm.team_id,
               p.id AS participant_id,
               p.display_name,
               tm.is_bench
          FROM scrim.team_members tm
          JOIN scrim.participants p ON p.id = tm.participant_id
         WHERE tm.team_id = ANY($1)
         ORDER BY tm.team_id ASC, tm.is_bench ASC, p.display_name ASC, p.id ASC
        "#,
    )
    .bind(team_ids)
    .fetch_all(pool)
    .await?;
    let mut members: HashMap<i32, Vec<MatchRequestMember>> = HashMap::new();
    for row in rows {
        members
            .entry(row.try_get("team_id")?)
            .or_default()
            .push(MatchRequestMember {
                participant_id: row.try_get("participant_id")?,
                display_name: row.try_get("display_name")?,
                is_bench: row.try_get("is_bench")?,
            });
    }
    Ok(members)
}

async fn load_match_request_responses(
    pool: &PgPool,
    request_ids: &[i32],
) -> DashboardDbResult<HashMap<(i32, i32), Vec<MatchRequestResponse>>> {
    if request_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT request_id, team_id, participant_id, slot_index, response
          FROM scrim.match_request_responses
         WHERE request_id = ANY($1)
        "#,
    )
    .bind(request_ids)
    .fetch_all(pool)
    .await?;
    let mut responses: HashMap<(i32, i32), Vec<MatchRequestResponse>> = HashMap::new();
    for row in rows {
        responses
            .entry((row.try_get("request_id")?, row.try_get("team_id")?))
            .or_default()
            .push(MatchRequestResponse {
                participant_id: row.try_get("participant_id")?,
                slot_index: row.try_get("slot_index")?,
                response: row.try_get("response")?,
            });
    }
    Ok(responses)
}

fn match_request_summary_json(
    request: &MatchRequestSummarySeed,
    members: &HashMap<i32, Vec<MatchRequestMember>>,
    responses: &HashMap<(i32, i32), Vec<MatchRequestResponse>>,
    deadline_passed: bool,
) -> Value {
    let slot_values = request.slot_options.as_array().cloned().unwrap_or_default();
    let mut slots = slot_values
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            json!({
                "index": index,
                "slot": slot,
                "available_count": 0,
                "starter_available_count": 0,
            })
        })
        .collect::<Vec<_>>();
    let mut teams = Vec::new();
    let mut missing_response_count = 0usize;
    let mut no_slot_count = 0usize;
    let mut slot_team_available_counts = vec![0usize; slots.len()];

    for (team_id, team_name) in [
        (request.team_a_id, Some(request.team_a_name.as_str())),
        (
            request.team_b_id.unwrap_or_default(),
            request.team_b_name.as_deref(),
        ),
    ] {
        if team_id == 0 {
            continue;
        }
        let team_members = members.get(&team_id).cloned().unwrap_or_default();
        let team_responses = responses
            .get(&(request.request_id, team_id))
            .cloned()
            .unwrap_or_default();
        let responded = team_responses
            .iter()
            .map(|response| response.participant_id)
            .collect::<BTreeSet<_>>();
        let mut team_slot_counts = vec![0usize; slots.len()];
        let mut team_missing = Vec::new();
        let mut team_no_slot = 0usize;
        let member_by_id = team_members
            .iter()
            .map(|member| (member.participant_id, member))
            .collect::<HashMap<_, _>>();

        for member in &team_members {
            if !responded.contains(&member.participant_id) {
                missing_response_count += 1;
                team_missing.push(json!({
                    "participant_id": member.participant_id,
                    "display_name": member.display_name,
                }));
            }
        }

        for response in &team_responses {
            if response.slot_index == -1 {
                if response.response == "unavailable" {
                    no_slot_count += 1;
                    team_no_slot += 1;
                }
                continue;
            }
            let slot_index = response.slot_index as usize;
            if slot_index >= slots.len() || response.response != "available" {
                continue;
            }
            team_slot_counts[slot_index] += 1;
            slots[slot_index]["available_count"] = json!(
                slots[slot_index]["available_count"]
                    .as_u64()
                    .unwrap_or_default()
                    + 1
            );
            if member_by_id
                .get(&response.participant_id)
                .is_some_and(|member| !member.is_bench)
            {
                slots[slot_index]["starter_available_count"] = json!(
                    slots[slot_index]["starter_available_count"]
                        .as_u64()
                        .unwrap_or_default()
                        + 1
                );
            }
        }
        for (index, available_count) in team_slot_counts.iter().enumerate() {
            if *available_count > 0 {
                slot_team_available_counts[index] += 1;
            }
        }

        teams.push(json!({
            "team_id": team_id,
            "team_name": team_name.unwrap_or("-"),
            "member_count": team_members.len(),
            "missing_response_count": team_missing.len(),
            "no_slot_count": team_no_slot,
            "missing": team_missing,
            "slots": team_slot_counts
                .into_iter()
                .enumerate()
                .map(|(index, available_count)| json!({
                    "index": index,
                    "available_count": available_count,
                }))
                .collect::<Vec<_>>(),
        }));
    }
    for (index, team_available_count) in slot_team_available_counts.iter().enumerate() {
        slots[index]["team_available_count"] = json!(team_available_count);
    }

    let recommended_slot_index = if deadline_passed {
        let replacement_need_counts = (0..slots.len())
            .map(|index| {
                match_request_replacement_needs(
                    request,
                    &slot_values,
                    members,
                    responses,
                    Some(index),
                    true,
                )
                .len()
            })
            .collect::<Vec<_>>();
        best_match_request_slot_index(&slots, &replacement_need_counts, teams.len())
    } else {
        None
    };
    let released_slot_index = request.released_slot_index.filter(|index| *index >= 0);
    let released_slot_index_usize =
        released_slot_index.and_then(|index| usize::try_from(index).ok());
    let released_slot = request
        .released_slot
        .clone()
        .or_else(|| released_slot_index_usize.and_then(|index| slot_values.get(index).cloned()));
    let is_override =
        released_slot_index_usize.is_some_and(|index| recommended_slot_index != Some(index));
    let selected_slot_index = if deadline_passed {
        released_slot_index_usize.or(recommended_slot_index)
    } else {
        None
    };
    let replacement_needs = match_request_replacement_needs(
        request,
        &slot_values,
        members,
        responses,
        selected_slot_index,
        deadline_passed,
    );
    let replacement_data_limited = deadline_passed && selected_slot_index.is_some();

    json!({
        "request_id": request.request_id,
        "status": request.status,
        "team_a": {
            "id": request.team_a_id,
            "name": request.team_a_name,
        },
        "team_b": request.team_b_id.map(|id| json!({
            "id": id,
            "name": request.team_b_name,
        })),
        "slots": slots,
        "teams": teams,
        "missing_response_count": missing_response_count,
        "no_slot_count": no_slot_count,
        "recommended_slot_index": recommended_slot_index,
        "selected_slot_index": selected_slot_index,
        "released_slot_index": released_slot_index,
        "released_slot": released_slot,
        "released_at": utc_to_json_unix(request.released_at),
        "released_by_user_id": request.released_by_user_id,
        "released_by_display_name": request.released_by_display_name,
        "override_reason": request.override_reason,
        "is_override": is_override,
        "replacement_needs": replacement_needs,
        "replacement_data_limited": replacement_data_limited,
        "replacement_data_note": if replacement_data_limited {
            Value::String(MATCH_REQUEST_REPLACEMENT_DATA_NOTE.to_string())
        } else {
            Value::Null
        },
    })
}

fn match_request_replacement_needs(
    request: &MatchRequestSummarySeed,
    slot_values: &[Value],
    members: &HashMap<i32, Vec<MatchRequestMember>>,
    responses: &HashMap<(i32, i32), Vec<MatchRequestResponse>>,
    selected_slot_index: Option<usize>,
    deadline_passed: bool,
) -> Vec<Value> {
    if !deadline_passed {
        return Vec::new();
    }
    let Some(slot_index) = selected_slot_index else {
        return Vec::new();
    };
    let Some(slot) = slot_values.get(slot_index) else {
        return Vec::new();
    };

    let mut needs = Vec::new();
    for (team_id, team_name) in [
        (request.team_a_id, Some(request.team_a_name.as_str())),
        (
            request.team_b_id.unwrap_or_default(),
            request.team_b_name.as_deref(),
        ),
    ] {
        if team_id == 0 {
            continue;
        }
        let team_responses = responses
            .get(&(request.request_id, team_id))
            .cloned()
            .unwrap_or_default();
        for member in members
            .get(&team_id)
            .into_iter()
            .flatten()
            .filter(|member| !member.is_bench)
        {
            let member_responses = team_responses
                .iter()
                .filter(|response| response.participant_id == member.participant_id)
                .collect::<Vec<_>>();
            if member_responses.iter().any(|response| {
                response.response == "available" && response.slot_index == slot_index as i32
            }) {
                continue;
            }
            let reason = if member_responses
                .iter()
                .any(|response| response.response == "unavailable" && response.slot_index == -1)
            {
                "Kein Slot passt"
            } else if member_responses.is_empty() {
                "Antwort fehlt"
            } else {
                "Für ausgewählten Slot nicht zugesagt"
            };
            needs.push(json!({
                "match_request_id": request.request_id,
                "team_id": team_id,
                "team_name": team_name.unwrap_or("-"),
                "slot_index": slot_index,
                "slot": slot,
                "participant_id": member.participant_id,
                "display_name": member.display_name,
                "missing_person": member.display_name,
                "role": null,
                "reason": reason,
                "data_limited": true,
            }));
        }
    }
    needs
}

fn best_match_request_slot_index(
    slots: &[Value],
    replacement_need_counts: &[usize],
    required_team_count: usize,
) -> Option<usize> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| {
            slot["available_count"].as_u64().unwrap_or_default() > 0
                && slot["team_available_count"].as_u64().unwrap_or_default() as usize
                    >= required_team_count
        })
        .max_by_key(|(index, slot)| {
            (
                slot["available_count"].as_u64().unwrap_or_default(),
                std::cmp::Reverse(
                    replacement_need_counts
                        .get(*index)
                        .copied()
                        .unwrap_or(usize::MAX),
                ),
                slot["starter_available_count"].as_u64().unwrap_or_default(),
                // max_by_key liefert bei Gleichstand das letzte Maximum; der Index kehrt das um,
                // damit bei komplettem Gleichstand die frühere Slotoption vorne bleibt.
                std::cmp::Reverse(*index),
            )
        })
        .map(|(index, _)| index)
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
    let Some(row) = row else {
        return Ok(None);
    };
    let mut scrim_match = match_json(row)?;
    attach_match_result_refs(pool, std::slice::from_mut(&mut scrim_match)).await?;
    Ok(Some(scrim_match))
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

enum MatchResultRefUpdate {
    Updated(Value),
    Duplicate,
    NotFound,
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

async fn add_match_result_ref(
    pool: &PgPool,
    match_id: i32,
    steam_match_id: i64,
    source_user_id: &str,
    source_display_name: &str,
) -> DashboardDbResult<MatchResultRefUpdate> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id
          FROM scrim.matches
         WHERE id = $1
         FOR UPDATE
        "#,
    )
    .bind(match_id)
    .fetch_optional(&mut *tx)
    .await?;
    if current.is_none() {
        return Ok(MatchResultRefUpdate::NotFound);
    }

    let inserted = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO scrim.match_result_refs(
            match_id, steam_match_id, source_user_id, source_display_name,
            fetch_status, entered_at, updated_at
        )
        VALUES($1, $2, $3, $4, 'pending', now(), now())
        ON CONFLICT (steam_match_id) DO NOTHING
        RETURNING id::bigint
        "#,
    )
    .bind(match_id)
    .bind(steam_match_id)
    .bind(source_user_id)
    .bind(source_display_name)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        return Ok(MatchResultRefUpdate::Duplicate);
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
    .bind(STATE_RESULT_REQUESTED)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(MatchResultRefUpdate::Updated(
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
    if lobby_state
        .as_deref()
        .is_some_and(|state| state_blocks_lobby_request(state, requested_state))
    {
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

fn state_blocks_lobby_request(current: &str, requested_state: &str) -> bool {
    !(requested_state == STATE_RESULT_REQUESTED
        && matches!(current, "in_progress" | "result_failed" | "finished"))
        && is_bot_owned_lobby_state(current)
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
    use dl_ai::{ChatProviderError, ChatResponse, MockChatProvider};
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

    async fn app_with_session_and_ai(
        ai: Arc<MockChatProvider>,
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
        let app = DashboardApp::new_with_ai(
            cfg,
            db.pool().clone(),
            Arc::new(NoMemberLookup),
            Arc::new(NoNameResolver),
            Some(ai),
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
        auth_mutation("POST", uri, session_id, csrf, body)
    }

    fn auth_mutation(
        method: &str,
        uri: &str,
        session_id: &str,
        csrf: &str,
        body: Value,
    ) -> Result<Request<Body>, axum::http::Error> {
        Request::builder()
            .method(method)
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

    async fn insert_team_member(
        pool: &PgPool,
        team_id: i32,
        participant_id: i32,
        display_name: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.participants(
                id, display_name, rank_source, status, source, created_at, updated_at
            )
            VALUES($1, $2, 'manual', 'assigned', 'test', now(), now())
            "#,
        )
        .bind(participant_id)
        .bind(display_name)
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.team_members(team_id, participant_id, role, is_captain, is_bench)
            VALUES($1, $2, 'player', false, false)
            "#,
        )
        .bind(team_id)
        .bind(participant_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match_request_response(
        pool: &PgPool,
        request_id: i32,
        team_id: i32,
        participant_id: i32,
        slot_index: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_responses(
                request_id, team_id, participant_id, discord_user_id, slot_index,
                response, source, responded_at, updated_at
            )
            VALUES($1, $2, $3, $4, $5, $6, 'button', now(), now())
            "#,
        )
        .bind(request_id)
        .bind(team_id)
        .bind(participant_id)
        .bind(i64::from(participant_id) + 10_000)
        .bind(slot_index)
        .bind(if slot_index == -1 {
            "unavailable"
        } else {
            "available"
        })
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_expired_match_request(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_batches(
                id, template, deadline_at, status, created_by_user_id,
                created_by_display_name, created_at, updated_at
            )
            VALUES(90, 'regular_scrim', now() - interval '1 hour', 'open', '42', 'Coach', now(), now())
            "#,
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_requests(
                id, batch_id, team_a_id, team_b_id, status, slot_options,
                created_at, updated_at
            )
            VALUES(
                91, 90, 1, 2, 'open',
                '[{"day":"sat","from":1200,"to":1320},{"day":"sun","from":1200,"to":1320}]'::jsonb,
                now(), now()
            )
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_future_match_request(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_batches(
                id, template, deadline_at, status, created_by_user_id,
                created_by_display_name, created_at, updated_at
            )
            VALUES(90, 'regular_scrim', now() + interval '1 hour', 'open', '42', 'Coach', now(), now())
            "#,
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_requests(
                id, batch_id, team_a_id, team_b_id, status, slot_options,
                created_at, updated_at
            )
            VALUES(
                91, 90, 1, 2, 'open',
                '[{"day":"sat","from":1200,"to":1320},{"day":"sun","from":1200,"to":1320}]'::jsonb,
                now(), now()
            )
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_lagebild_seed(pool: &PgPool, evidence_url: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO scrim.lagebild_snapshots(
                id, team_id, generated_for, source, status, lagebild_text, data_summary,
                model, generated_at, created_at
            )
            OVERRIDING SYSTEM VALUE
            VALUES(
                7001, 1, 'weekly', 'ai', 'ok',
                'Die Lage wirkt aktuell okay.\n\nEvidenzen:\n- [Terminabfrage](https://discord.com/channels/1289721245281292288/100/9001)',
                '{"data_limited":true}'::jsonb, 'mock', now() - INTERVAL '1 hour', now() - INTERVAL '1 hour'
            )
            "#,
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.lagebild_evidences(
                snapshot_id, evidence_type, label, url, occurred_at, payload
            )
            VALUES(
                7001, 'match_request', 'Terminabfrage',
                $1,
                now(), '{"request_id":91}'::jsonb
            )
            "#,
        )
        .bind(evidence_url)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_participant_for_notes(pool: &PgPool, id: i32) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO scrim.participants(
                 id, display_name, rank_source, status, source, created_at, updated_at
             )
             VALUES($1, 'Notes Test', 'manual', 'new', 'test', now(), now())",
        )
        .bind(id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn participant_notes(pool: &PgPool, id: i32) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT notes FROM scrim.participants WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
    }

    #[tokio::test]
    async fn dashboard_roster_schreibt_nur_im_legacy_runtime(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (legacy_db, legacy_app, session_id, csrf) = app_with_session().await?;
        insert_participant_for_notes(legacy_db.pool(), 1).await?;
        let response = legacy_app
            .oneshot(auth_post(
                "/api/scrims/participants/1/notes",
                &session_id,
                &csrf,
                json!({ "notes": "legacy" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            participant_notes(legacy_db.pool(), 1).await?.as_deref(),
            Some("legacy")
        );

        let (turniere_db, turniere_app, session_id, csrf) = app_with_session().await?;
        insert_participant_for_notes(turniere_db.pool(), 2).await?;
        dl_central_db::testing::set_scrim_runtime_turniere(turniere_db.pool()).await?;
        let response = turniere_app
            .oneshot(auth_post(
                "/api/scrims/participants/2/notes",
                &session_id,
                &csrf,
                json!({ "notes": "turniere" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        assert_eq!(body.as_ref(), SCRIM_RUNTIME_DENIED_MESSAGE.as_bytes());
        assert_eq!(participant_notes(turniere_db.pool(), 2).await?, None);

        let (inconsistent_db, inconsistent_app, session_id, csrf) = app_with_session().await?;
        insert_participant_for_notes(inconsistent_db.pool(), 3).await?;
        dl_central_db::testing::set_scrim_runtime_inconsistent(inconsistent_db.pool()).await?;
        let response = inconsistent_app
            .oneshot(auth_post(
                "/api/scrims/participants/3/notes",
                &session_id,
                &csrf,
                json!({ "notes": "widerspruch" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(participant_notes(inconsistent_db.pool(), 3).await?, None);
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
        assert_eq!(data["presets"][0]["key"], "weekend_evening");
        assert_eq!(data["presets"][0]["name"], "Wochenende abends");
        assert_eq!(data["presets"][0]["slots"][0]["day"], "sat");
        assert_eq!(data["presets"][0]["slots"][0]["from"], 20 * 60);
        assert_eq!(data["presets"][0]["slots"][0]["to"], 22 * 60);
        assert_eq!(data["presets"][0]["slots"][1]["day"], "sun");
        assert_eq!(data["presets"][0]["slots"][1]["from"], 20 * 60);
        assert_eq!(data["presets"][0]["slots"][1]["to"], 22 * 60);
        Ok(())
    }

    #[tokio::test]
    async fn slot_preset_anlegen_erscheint_in_defaults() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, session_id, csrf) = app_with_session().await?;

        let response = app
            .clone()
            .oneshot(auth_post(
                "/api/scrims/slot-presets",
                &session_id,
                &csrf,
                json!({
                    "name": "Werktags",
                    "slots": [
                        { "day": "tue", "from": 18 * 60, "to": 20 * 60 },
                        { "day": "thu", "from": 19 * 60, "to": 21 * 60 }
                    ]
                }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let created: Value = serde_json::from_slice(&body)?;
        assert_eq!(created["preset"]["name"], "Werktags");
        assert_eq!(created["preset"]["created_by_user_id"], "42");
        let preset_id = created["preset"]["id"]
            .as_i64()
            .ok_or("missing preset id")?;

        let response = app
            .clone()
            .oneshot(auth_get(
                "/api/scrims/match-requests/defaults",
                &session_id,
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let defaults: Value = serde_json::from_slice(&body)?;
        assert!(defaults["presets"]
            .as_array()
            .is_some_and(|presets| presets.iter().any(|preset| {
                preset["id"] == created["preset"]["id"] && preset["slots"][0]["day"] == "tue"
            })));

        let response = app
            .clone()
            .oneshot(auth_mutation(
                "PUT",
                &format!("/api/scrims/slot-presets/{preset_id}"),
                &session_id,
                &csrf,
                json!({
                    "name": "Spät",
                    "slots": [
                        { "day": "fri", "from": 20 * 60, "to": 22 * 60 },
                        { "day": "sat", "from": 21 * 60, "to": 23 * 60 }
                    ]
                }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let updated: Value = serde_json::from_slice(&body)?;
        assert_eq!(updated["preset"]["name"], "Spät");
        assert_eq!(updated["preset"]["slots"][0]["day"], "fri");

        let response = app
            .clone()
            .oneshot(auth_mutation(
                "DELETE",
                &format!("/api/scrims/slot-presets/{preset_id}"),
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(auth_get(
                "/api/scrims/match-requests/defaults",
                &session_id,
            )?)
            .await?;
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let defaults: Value = serde_json::from_slice(&body)?;
        assert!(!defaults["presets"]
            .as_array()
            .is_some_and(|presets| presets.iter().any(|preset| preset["id"] == preset_id)));
        Ok(())
    }

    #[tokio::test]
    async fn slot_preset_api_verwendet_block_slot_validierung(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, session_id, csrf) = app_with_session().await?;
        let invalid_slots = [
            json!([
                { "day": "invalid", "from": 18 * 60, "to": 20 * 60 },
                { "day": "thu", "from": 19 * 60, "to": 21 * 60 }
            ]),
            json!([
                { "day": "tue", "from": -1, "to": 20 * 60 },
                { "day": "thu", "from": 19 * 60, "to": 21 * 60 }
            ]),
            json!([
                { "day": "tue", "from": 20 * 60, "to": 18 * 60 },
                { "day": "thu", "from": 19 * 60, "to": 21 * 60 }
            ]),
        ];

        for slots in invalid_slots {
            let response = app
                .clone()
                .oneshot(auth_post(
                    "/api/scrims/slot-presets",
                    &session_id,
                    &csrf,
                    json!({ "name": "Ungültig", "slots": slots }),
                )?)
                .await?;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        Ok(())
    }

    #[tokio::test]
    async fn scrims_overview_liefert_zwei_wochen_blockvorschlag_mit_rotation(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team(db.pool(), 3, "C").await?;
        insert_team(db.pool(), 4, "D").await?;

        let response = app.oneshot(auth_get("/api/scrims", &session_id)?).await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let block = &data["suggested_block"];
        assert_eq!(block["key"], "two_week_scrim_block");
        assert_eq!(block["rhythm_days"], 14);
        assert_eq!(block["template"], "regular_scrim");
        assert_eq!(block["slots"][0]["day"], "sat");
        assert_eq!(block["matches"][0]["team_a_id"], 1);
        assert_eq!(block["matches"][0]["team_b_id"], 3);
        assert_eq!(block["matches"][1]["team_a_id"], 2);
        assert_eq!(block["matches"][1]["team_b_id"], 4);
        Ok(())
    }

    #[tokio::test]
    async fn scrims_overview_liefert_lagebild_timeline_mit_evidenzen(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_lagebild_seed(
            db.pool(),
            "https://discord.com/channels/1289721245281292288/100/9001",
        )
        .await?;

        let response = app.oneshot(auth_get("/api/scrims", &session_id)?).await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let first = &data["lagebilder"][0];
        assert_eq!(first["team_id"], 1);
        assert_eq!(first["team_name"], "A");
        assert!(first["current"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Die Lage wirkt aktuell okay."));
        assert_eq!(
            first["timeline"][0]["evidences"][0]["label"],
            "Terminabfrage"
        );
        assert_eq!(
            first["timeline"][0]["evidences"][0]["url"],
            "https://discord.com/channels/1289721245281292288/100/9001"
        );
        Ok(())
    }

    #[tokio::test]
    async fn scrims_overview_gibt_nur_discord_evidenzlinks_aus(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_lagebild_seed(db.pool(), "javascript:alert(1)").await?;

        let response = app.oneshot(auth_get("/api/scrims", &session_id)?).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(
            data["lagebilder"][0]["current"]["evidences"][0]["url"],
            Value::Null
        );
        Ok(())
    }

    #[tokio::test]
    async fn lagebild_korrektur_route_speichert_chat_und_aktualisiert_letzten_stand(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockChatProvider::new(vec![Ok(ChatResponse::text(
            r#"{"reply":"Überarbeitet.","lagebild":"Korrigierte Lage.\n\nEvidenzen:\n- [Terminabfrage](https://discord.com/channels/1289721245281292288/100/9001)"}"#,
        ))]);
        let (db, app, session_id, csrf) = app_with_session_and_ai(provider).await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_lagebild_seed(
            db.pool(),
            "https://discord.com/channels/1289721245281292288/100/9001",
        )
        .await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/teams/1/lagebild/corrections",
                &session_id,
                &csrf,
                json!({ "message": "A2 hat inzwischen zugesagt." }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["assistant_message"]["message"], "Überarbeitet.");
        assert!(data["current"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Korrigierte Lage"));

        let correction_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM scrim.lagebild_corrections")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(correction_count, 2);
        let snapshot_source: String = sqlx::query_scalar(
            "SELECT source FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY generated_at DESC, id DESC LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(snapshot_source, "correction");
        let ledger_decision: String = sqlx::query_scalar(
            "SELECT decision FROM bot.ai_decision_ledger WHERE source = 'scrim.lagebild.correction' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(ledger_decision, "yes");
        Ok(())
    }

    #[tokio::test]
    async fn lagebild_korrektur_rollt_ai_daten_ohne_entscheidungslog_zurueck(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockChatProvider::new(vec![Ok(ChatResponse::text(
            r#"{"reply":"Überarbeitet.","lagebild":"Korrigierte Lage."}"#,
        ))]);
        let (db, app, session_id, csrf) = app_with_session_and_ai(provider).await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_lagebild_seed(
            db.pool(),
            "https://discord.com/channels/1289721245281292288/100/9001",
        )
        .await?;
        sqlx::query(
            "ALTER TABLE bot.ai_decision_ledger ADD CONSTRAINT reject_scrim_correction_test CHECK (source <> 'scrim.lagebild.correction')",
        )
        .execute(db.pool())
        .await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/teams/1/lagebild/corrections",
                &session_id,
                &csrf,
                json!({ "message": "A2 hat inzwischen zugesagt." }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let correction_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM scrim.lagebild_corrections")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(correction_count, 1, "nur die menschliche Eingabe bleibt");
        let corrected_snapshot_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scrim.lagebild_snapshots WHERE team_id = 1 AND source = 'correction'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(corrected_snapshot_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn lagebild_korrektur_loggt_timeout_als_timeout() -> Result<(), Box<dyn std::error::Error>>
    {
        let provider = MockChatProvider::new(vec![Err(ChatProviderError::Timeout)]);
        let (db, app, session_id, csrf) = app_with_session_and_ai(provider).await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_lagebild_seed(
            db.pool(),
            "https://discord.com/channels/1289721245281292288/100/9001",
        )
        .await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/teams/1/lagebild/corrections",
                &session_id,
                &csrf,
                json!({ "message": "Bitte erneut prüfen." }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let decision: String = sqlx::query_scalar(
            "SELECT decision FROM bot.ai_decision_ledger WHERE source = 'scrim.lagebild.correction' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(decision, "timeout");
        Ok(())
    }

    #[tokio::test]
    async fn lagebild_korrektur_loggt_unklare_ai_antwort_als_unsicher(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockChatProvider::new(vec![Ok(ChatResponse::text("kein JSON"))]);
        let (db, app, session_id, csrf) = app_with_session_and_ai(provider).await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_lagebild_seed(
            db.pool(),
            "https://discord.com/channels/1289721245281292288/100/9001",
        )
        .await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/teams/1/lagebild/corrections",
                &session_id,
                &csrf,
                json!({ "message": "Das ist unklar." }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let decision: String = sqlx::query_scalar(
            "SELECT decision FROM bot.ai_decision_ledger WHERE source = 'scrim.lagebild.correction' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(decision, "unsure");
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
    async fn match_request_summary_empfiehlt_nach_frist_bestgemeinsamen_slot(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 1, 102, "A2").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_team_member(db.pool(), 2, 202, "B2").await?;
        insert_team_member(db.pool(), 2, 203, "B3").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        insert_match_request_response(db.pool(), 91, 1, 102, -1).await?;
        insert_match_request_response(db.pool(), 91, 2, 201, 0).await?;
        insert_match_request_response(db.pool(), 91, 2, 202, 1).await?;

        let response = app
            .oneshot(auth_get(
                "/api/scrims/match-requests/90/summary",
                &session_id,
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let first_match = &data["matches"][0];
        assert_eq!(data["batch"]["id"], 90);
        assert_eq!(data["deadline_passed"], true);
        assert_eq!(first_match["recommended_slot_index"], 0);
        assert_eq!(first_match["slots"][0]["available_count"], 2);
        assert_eq!(first_match["slots"][1]["available_count"], 1);
        assert_eq!(first_match["missing_response_count"], 1);
        assert_eq!(first_match["no_slot_count"], 1);
        Ok(())
    }

    #[test]
    fn match_request_slot_empfiehlt_weniger_ersatzbedarf_vor_mehr_stammspielern() {
        let slots = vec![
            json!({
                "available_count": 4,
                "team_available_count": 2,
                "starter_available_count": 1,
            }),
            json!({
                "available_count": 4,
                "team_available_count": 2,
                "starter_available_count": 2,
            }),
        ];

        assert_eq!(best_match_request_slot_index(&slots, &[0, 1], 2), Some(0));
    }

    #[test]
    fn match_request_slot_bleibt_bei_gleichstand_auf_der_frueheren_option() {
        let slots = vec![
            json!({
                "available_count": 4,
                "team_available_count": 2,
                "starter_available_count": 2,
            }),
            json!({
                "available_count": 4,
                "team_available_count": 2,
                "starter_available_count": 2,
            }),
        ];

        assert_eq!(best_match_request_slot_index(&slots, &[1, 1], 2), Some(0));
    }

    #[tokio::test]
    async fn overview_liefert_match_request_auswertungen() -> Result<(), Box<dyn std::error::Error>>
    {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        insert_match_request_response(db.pool(), 91, 2, 201, 0).await?;

        let response = app.oneshot(auth_get("/api/scrims", &session_id)?).await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let summaries = data["match_request_summaries"]
            .as_array()
            .ok_or("missing summaries")?;
        assert_eq!(summaries[0]["batch"]["id"], 90);
        assert_eq!(summaries[0]["matches"][0]["recommended_slot_index"], 0);
        Ok(())
    }

    #[tokio::test]
    async fn match_request_summary_empfiehlt_nur_slots_mit_zusage_beider_teams(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        insert_match_request_response(db.pool(), 91, 2, 201, 1).await?;

        let response = app
            .oneshot(auth_get(
                "/api/scrims/match-requests/90/summary",
                &session_id,
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["matches"][0]["recommended_slot_index"], Value::Null);
        Ok(())
    }

    #[tokio::test]
    async fn match_request_summary_zeigt_ersatzbedarf_nur_nach_frist_fuer_gewaehlten_slot(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 1, 102, "A2").await?;
        insert_team_member(db.pool(), 1, 103, "A3").await?;
        insert_team_member(db.pool(), 1, 104, "A4 Bench").await?;
        sqlx::query(
            "UPDATE scrim.team_members SET is_bench = true WHERE team_id = 1 AND participant_id = 104",
        )
        .execute(db.pool())
        .await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_team_member(db.pool(), 2, 202, "B2").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        insert_match_request_response(db.pool(), 91, 1, 102, -1).await?;
        insert_match_request_response(db.pool(), 91, 2, 201, 0).await?;
        insert_match_request_response(db.pool(), 91, 2, 202, 1).await?;

        let response = app
            .oneshot(auth_get(
                "/api/scrims/match-requests/90/summary",
                &session_id,
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let first_match = &data["matches"][0];
        assert_eq!(first_match["selected_slot_index"], 0);
        assert_eq!(first_match["replacement_data_limited"], true);
        assert_eq!(
            first_match["replacement_data_note"],
            "Rollen/Lineup-Daten fehlen; angezeigt werden nur konkrete Personen aus Teammitgliedern und Antworten."
        );
        assert_eq!(
            first_match["replacement_needs"]
                .as_array()
                .expect("replacement_needs array")
                .len(),
            3
        );
        assert_eq!(first_match["replacement_needs"][0]["display_name"], "A2");
        assert_eq!(
            first_match["replacement_needs"][0]["reason"],
            "Kein Slot passt"
        );
        assert_eq!(first_match["replacement_needs"][1]["display_name"], "A3");
        assert_eq!(
            first_match["replacement_needs"][1]["reason"],
            "Antwort fehlt"
        );
        assert_eq!(first_match["replacement_needs"][2]["display_name"], "B2");
        assert_eq!(
            first_match["replacement_needs"][2]["reason"],
            "Für ausgewählten Slot nicht zugesagt"
        );
        Ok(())
    }

    #[tokio::test]
    async fn match_request_summary_zeigt_vor_frist_keinen_ersatzbedarf(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, _csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_future_match_request(db.pool()).await?;

        let response = app
            .oneshot(auth_get(
                "/api/scrims/match-requests/90/summary",
                &session_id,
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        let first_match = &data["matches"][0];
        assert_eq!(data["deadline_passed"], false);
        assert_eq!(first_match["selected_slot_index"], Value::Null);
        assert_eq!(first_match["replacement_needs"], json!([]));
        Ok(())
    }

    #[tokio::test]
    async fn match_request_release_route_gibt_empfohlenen_slot_frei(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        insert_match_request_response(db.pool(), 91, 2, 201, 0).await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/match-requests/91/release",
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["request"]["recommended_slot_index"], 0);
        assert_eq!(data["request"]["released_slot_index"], 0);
        assert_eq!(data["request"]["override_reason"], Value::Null);

        let row = sqlx::query(
            r#"
            SELECT released_slot_index, override_reason, released_by_display_name
              FROM scrim.match_requests
             WHERE id = 91
            "#,
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(
            row.try_get::<Option<i32>, _>("released_slot_index")?,
            Some(0)
        );
        assert_eq!(row.try_get::<Option<String>, _>("override_reason")?, None);
        assert_eq!(
            row.try_get::<Option<String>, _>("released_by_display_name")?,
            Some("Coach".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn match_request_release_route_speichert_override_slot_und_grund(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        insert_match_request_response(db.pool(), 91, 2, 201, 0).await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/match-requests/91/release",
                &session_id,
                &csrf,
                json!({
                    "slot_index": 1,
                    "reason": "Team B kann Sonntag leichter vollzaehlig."
                }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["request"]["recommended_slot_index"], 0);
        assert_eq!(data["request"]["released_slot_index"], 1);
        assert_eq!(
            data["request"]["override_reason"],
            "Team B kann Sonntag leichter vollzaehlig."
        );
        assert_eq!(data["request"]["released_by_display_name"], "Coach");

        let row = sqlx::query(
            r#"
            SELECT released_slot_index, override_reason, released_by_user_id, released_by_display_name
              FROM scrim.match_requests
             WHERE id = 91
            "#,
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(
            row.try_get::<Option<i32>, _>("released_slot_index")?,
            Some(1)
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("override_reason")?,
            Some("Team B kann Sonntag leichter vollzaehlig.".to_string())
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("released_by_user_id")?,
            Some("42".to_string())
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("released_by_display_name")?,
            Some("Coach".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn match_request_reminder_route_speichert_freigabe_fuer_fehlende_stimmen(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 1, 102, "A2").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_expired_match_request(db.pool()).await?;
        insert_match_request_response(db.pool(), 91, 1, 101, 0).await?;
        sqlx::query(
            r#"
            UPDATE scrim.teams
               SET discord_channel_id = CASE id WHEN 1 THEN 100 WHEN 2 THEN 200 END
             WHERE id IN (1, 2)
            "#,
        )
        .execute(db.pool())
        .await?;
        sqlx::query(
            r#"
            UPDATE scrim.participants
               SET discord_id = CASE id WHEN 101 THEN 555 WHEN 102 THEN 666 WHEN 201 THEN 777 END
             WHERE id IN (101, 102, 201)
            "#,
        )
        .execute(db.pool())
        .await?;
        sqlx::query(
            r#"
            UPDATE scrim.match_requests
               SET team_query_message_ids = '{"1":{"channel_id":100,"message_id":9001},"2":{"channel_id":200,"message_id":9002}}'::jsonb
             WHERE id = 91
            "#,
        )
        .execute(db.pool())
        .await?;

        let response = app
            .clone()
            .oneshot(auth_post(
                "/api/scrims/match-requests/91/reminders",
                &session_id,
                &csrf,
                json!({ "team_id": 1, "template": "frist_bald" }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["reminder"]["request_id"], 91);
        assert_eq!(data["reminder"]["team_id"], 1);
        assert_eq!(data["reminder"]["template"], "frist_bald");
        assert_eq!(data["reminder"]["target_kind"], "members");
        assert_eq!(data["reminder"]["missing_count"], 1);

        let row = sqlx::query(
            r#"
            SELECT request_id, team_id, template, target_kind, target_participant_ids,
                   approved_by_user_id, approved_by_display_name, status
              FROM scrim.match_request_reminders
             WHERE id = $1
            "#,
        )
        .bind(i64_to_i32(
            data["reminder"]["id"]
                .as_i64()
                .ok_or("missing reminder id")?,
            "id",
        )?)
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.try_get::<i32, _>("request_id")?, 91);
        assert_eq!(row.try_get::<i32, _>("team_id")?, 1);
        assert_eq!(row.try_get::<String, _>("template")?, "frist_bald");
        assert_eq!(row.try_get::<String, _>("target_kind")?, "members");
        assert_eq!(
            row.try_get::<Vec<i32>, _>("target_participant_ids")?,
            vec![102]
        );
        assert_eq!(row.try_get::<String, _>("approved_by_user_id")?, "42");
        assert_eq!(
            row.try_get::<String, _>("approved_by_display_name")?,
            "Coach"
        );
        assert_eq!(row.try_get::<String, _>("status")?, "approved");

        let response = app
            .oneshot(auth_post(
                "/api/scrims/match-requests/91/reminders",
                &session_id,
                &csrf,
                json!({ "team_id": 2 }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["reminder"]["template"], "antwort_fehlt");
        Ok(())
    }

    #[tokio::test]
    async fn match_request_reminder_route_lehnt_unbekannte_vorlage_ab(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, session_id, csrf) = app_with_session().await?;
        let response = app
            .oneshot(auth_post(
                "/api/scrims/match-requests/91/reminders",
                &session_id,
                &csrf,
                json!({ "team_id": 1, "template": "frei_formuliert" }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn match_request_reminder_route_blockiert_teamziel_ohne_rolle(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_team_member(db.pool(), 1, 101, "A1").await?;
        insert_team_member(db.pool(), 2, 201, "B1").await?;
        insert_expired_match_request(db.pool()).await?;
        sqlx::query("UPDATE scrim.teams SET discord_channel_id = 100 WHERE id = 1")
            .execute(db.pool())
            .await?;
        sqlx::query(
            "UPDATE scrim.match_requests SET team_query_message_ids = '{\"1\":{\"channel_id\":100,\"message_id\":9001}}'::jsonb WHERE id = 91",
        )
        .execute(db.pool())
        .await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/match-requests/91/reminders",
                &session_id,
                &csrf,
                json!({ "team_id": 1 }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
    async fn start_route_prueft_auth_bevor_deaktiviert_antwortet(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, app, _session_id, _csrf) = app_with_session().await?;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/scrims/matches/10/start")
                    .body(Body::empty())?,
            )
            .await?;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
    async fn result_route_erlaubt_manuellen_fallback_fuer_laufendes_match(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 14, "in_progress").await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/matches/14/result",
                &session_id,
                &csrf,
                json!({}),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let state =
            sqlx::query_scalar::<_, String>("SELECT lobby_state FROM scrim.matches WHERE id = 14")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(state, "result_requested");
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

    #[tokio::test]
    async fn match_id_route_speichert_eingabe_und_blockiert_duplikate(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 20, "draft").await?;
        insert_match(db.pool(), 21, "draft").await?;

        let response = app
            .clone()
            .oneshot(auth_post(
                "/api/scrims/matches/20/match-ids",
                &session_id,
                &csrf,
                json!({ "match_id": "987654321" }),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await?;
        let data: Value = serde_json::from_slice(&body)?;
        assert_eq!(data["match"]["id"], 20);
        assert_eq!(data["match"]["lobby_state"], "result_requested");
        assert_eq!(data["match"]["steam_match_id"], Value::Null);
        assert_eq!(
            data["match"]["match_result_refs"][0]["steam_match_id"],
            987654321
        );
        assert_eq!(
            data["match"]["match_result_refs"][0]["fetch_status"],
            "pending"
        );
        assert_eq!(
            data["match"]["match_result_refs"][0]["source_display_name"],
            "Coach"
        );

        let row = sqlx::query(
            r#"
            SELECT m.steam_match_id,
                   m.lobby_state,
                   r.fetch_status,
                   r.source_user_id,
                   r.source_display_name
              FROM scrim.matches m
              JOIN scrim.match_result_refs r ON r.match_id = m.id
             WHERE m.id = 20
            "#,
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.try_get::<Option<i64>, _>("steam_match_id")?, None);
        assert_eq!(
            row.try_get::<Option<String>, _>("lobby_state")?,
            Some("result_requested".to_string())
        );
        assert_eq!(row.try_get::<String, _>("fetch_status")?, "pending");
        assert_eq!(row.try_get::<String, _>("source_user_id")?, "42");
        assert_eq!(row.try_get::<String, _>("source_display_name")?, "Coach");

        let duplicate = app
            .oneshot(auth_post(
                "/api/scrims/matches/21/match-ids",
                &session_id,
                &csrf,
                json!({ "match_id": 987654321 }),
            )?)
            .await?;
        assert_eq!(duplicate.status(), StatusCode::CONFLICT);
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scrim.match_result_refs WHERE steam_match_id = 987654321",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn neue_match_id_ueberschreibt_erfolgreiche_primaere_id_nicht(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (db, app, session_id, csrf) = app_with_session().await?;
        insert_team(db.pool(), 1, "A").await?;
        insert_team(db.pool(), 2, "B").await?;
        insert_match(db.pool(), 22, "finished").await?;
        sqlx::query(
            "UPDATE scrim.matches SET steam_match_id = 111, result_json = '{}'::jsonb WHERE id = 22",
        )
        .execute(db.pool())
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_result_refs(
                match_id, steam_match_id, source_user_id, source_display_name,
                fetch_status, entered_at, fetched_at, updated_at
            )
            VALUES(22, 111, '42', 'Coach', 'fetched', now(), now(), now())
            "#,
        )
        .execute(db.pool())
        .await?;

        let response = app
            .oneshot(auth_post(
                "/api/scrims/matches/22/match-ids",
                &session_id,
                &csrf,
                json!({ "match_id": 222 }),
            )?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let primary_id: Option<i64> =
            sqlx::query_scalar("SELECT steam_match_id FROM scrim.matches WHERE id = 22")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(primary_id, Some(111));
        Ok(())
    }
}
