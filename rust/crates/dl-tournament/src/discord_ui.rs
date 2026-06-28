//! Turnier-Discord-UI (User-Flow) — Port von `cogs/customgames/turnier.py`.
//!
//! Panel-Buttons mit Original-IDs (`turnier_panel_anmelden/abmelden/status`):
//! Anmelden prüft Turnier-Rolle, offenen Zeitraum und Steam-Verknüpfung,
//! zeigt den verifizierten Rang und bietet Solo- oder Team-Anmeldung
//! (Team-Auswahl mit Voll-Markierung bzw. Team-Neuerstellung per Formular).
//! Der Admin-Flow (Zeiträume/Teams/Clear/Panel-Posten) ist ebenfalls
//! zustandslos über `tnadm:*`-IDs verdrahtet — die Datenbasis ist identisch
//! (dl-tournament::store).
//!
//! Die Auswahl-Menüs des Originals sind RAM-Views (120-s-Timeout) — Rust
//! vergibt stabile `tn:*`-IDs mit eingebettetem Besitzer; sie überleben
//! damit auch Neustarts.

use std::sync::Arc;

use dl_discord::interactions::{ChannelMessage, ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Value};

use crate::store::{self, TournamentStore};

pub const TURNIER_ROLE_ID: u64 = 1474210107255554331;
pub const TEAM_MAX_SIZE: i64 = store::TEAM_MAX_SIZE;
const SIGNUP_PAGE_SIZE: usize = 10;

// --- Turnier-Admin UI-Texte (deutsch) ---
const ADMIN_DASH_TITLE: &str = "Turnier-Verwaltung";
const ADMIN_DASH_PERIOD_FIELD: &str = "Aktiver Zeitraum";
const ADMIN_DASH_STATUS_FIELD: &str = "Status";
const ADMIN_DASH_END_FIELD: &str = "Anmeldeschluss";
const ADMIN_DASH_NO_ACTIVE_PERIOD_DESC: &str =
    "Aktuell läuft kein Anmeldezeitraum. Erstelle einen, um Anmeldungen zu öffnen.";
const ADMIN_DASH_SUMMARY_FIELD: &str = "Übersicht";
const ADMIN_DASH_SUMMARY_LABEL: &str = "Anmeldungen | Solo | im Team | Teams";
const BTN_PERIOD_CREATE: &str = "Zeitraum erstellen";
const BTN_PERIOD_CLOSE: &str = "Zeitraum beenden";
const BTN_SIGNUPS_MANAGE: &str = "Anmeldungen verwalten";
const BTN_TEAMS_MANAGE: &str = "Teams verwalten";
const BTN_PANEL_POST: &str = "Panel posten";
const BTN_CLEAR_OPEN: &str = "Alle Anmeldungen löschen";
const BTN_ADMIN_DASHBOARD: &str = "Turnier-Verwaltung";
const PERIOD_CREATE_MODAL_TITLE: &str = "Anmeldezeitraum erstellen";
const PERIOD_NAME_LABEL: &str = "Name";
const PERIOD_NAME_PLACEHOLDER: &str = "3–64 Zeichen, z. B. Sommer-Cup 2026";
const PERIOD_START_LABEL: &str = "Anmeldestart";
const PERIOD_START_PLACEHOLDER: &str = "TT.MM.JJJJ HH:MM, z. B. 15.07.2026 18:00";
const PERIOD_END_LABEL: &str = "Anmeldeschluss";
const PERIOD_END_PLACEHOLDER: &str = "TT.MM.JJJJ HH:MM, z. B. 22.07.2026 23:59";
const PERIOD_TEAM_SIZE_LABEL: &str = "Teamgröße";
const PERIOD_TEAM_SIZE_PLACEHOLDER: &str = "2–20, leer = 6";
const TEAM_CREATE_MODAL_TITLE: &str = "Team erstellen";
const TEAM_NAME_LABEL: &str = "Teamname";
const TEAM_NAME_PLACEHOLDER: &str = "z. B. Team Alpha";
const SIGNUP_REMOVE_SELECT_PLACEHOLDER: &str = "Anmeldung zum Entfernen wählen";
const BTN_TEAM_CREATE: &str = "Team erstellen";
const BTN_TEAM_DELETE_OPEN: &str = "Team löschen";
const BTN_CLEAR_CONFIRM_YES: &str = "Ja, alle löschen";
const BTN_CLEAR_CONFIRM_NO: &str = "Abbrechen";
const ADMIN_DASHBOARD_HINT: &str =
    "\nAdmin: Über die Turnier-Verwaltung steuerst du Zeiträume, Anmeldungen und Teams.";
const ADMIN_ERR_INVALID_CONTEXT: &str = "Diese Aktion ist ungültig oder abgelaufen.";
const ADMIN_ERR_OWNER_ONLY: &str =
    "Dieses Menü gehört jemand anderem. Öffne die Turnier-Verwaltung selbst.";
const ADMIN_ERR_PERMISSION_REQUIRED: &str = "Dafür fehlen dir die nötigen Rechte (Server verwalten).";
const SIGNUP_REMOVE_INVALID_SELECTION: &str = "Ungültige Auswahl.";
const SIGNUP_REMOVE_SUCCESS: &str = "Anmeldung entfernt.";
const SIGNUP_REMOVE_NOT_FOUND: &str = "Diese Anmeldung gibt es nicht mehr.";
const CLEAR_CONFIRM_PROMPT: &str =
    "Wirklich **alle** Anmeldungen löschen? Das lässt sich nicht rückgängig machen.";
const CLEAR_SUCCESS: &str = "Alle Anmeldungen wurden gelöscht.";
const CLEAR_ERROR: &str = "Konnte die Anmeldungen nicht löschen. Versuch es später nochmal.";
const CLEAR_CANCELLED: &str = "Abgebrochen – es wurde nichts gelöscht.";
const ADMIN_ERR_UNKNOWN_ACTION: &str = "Unbekannte Aktion.";
const PERIOD_CREATE_INVALID_INPUT: &str =
    "Eingaben ungültig: Name 3–64 Zeichen, Datum als TT.MM.JJJJ HH:MM (Ende nach Start), Teamgröße 2–20.";
const PERIOD_CREATE_SUCCESS: &str = "Anmeldezeitraum erstellt.";
const PERIOD_CREATE_ERROR: &str = "Konnte den Zeitraum nicht erstellen. Versuch es später nochmal.";
const PERIOD_CLOSE_NO_ACTIVE_PERIOD: &str = "Es läuft gerade kein Anmeldezeitraum.";
const PERIOD_CLOSE_INVALID_ACTIVE_PERIOD: &str =
    "Der aktive Zeitraum ist beschädigt – bitte neu erstellen.";
const PERIOD_CLOSE_SUCCESS: &str = "Anmeldezeitraum beendet.";
const PERIOD_CLOSE_ERROR: &str = "Konnte den Zeitraum nicht beenden. Versuch es später nochmal.";
const SIGNUPS_EMPTY_REPLY: &str = "Es liegen keine Anmeldungen vor.";
const SIGNUPS_LIST_TITLE: &str = "Anmeldungen";
const SIGNUPS_LIST_EMPTY_DESC: &str = "Auf dieser Seite stehen keine Anmeldungen.";
const SIGNUPS_LIST_FOOTER_LABEL: &str = "Seite";
const TEAMS_LIST_TITLE: &str = "Teams";
const TEAMS_EMPTY_DESC: &str = "Noch keine Teams angelegt.";
const TEAMS_LIST_TRUNCATED_FOOTER: &str = "Es werden nur die ersten 20 Teams angezeigt.";
const TEAM_CREATE_SUCCESS: &str = "Team erstellt.";
const TEAM_CREATE_ERROR: &str = "Konnte das Team nicht erstellen. Versuch es später nochmal.";
const TEAM_DELETE_NO_TEAMS: &str = "Es gibt keine Teams zum Löschen.";
const TEAM_DELETE_OPTION_META_LABEL: &str = "ID | Mitglieder";
const TEAM_DELETE_SELECT_PROMPT: &str = "Welches Team soll gelöscht werden?";
const TEAM_DELETE_SELECT_PLACEHOLDER: &str = "Team zum Löschen wählen";
const TEAM_DELETE_INVALID_SELECTION: &str = "Ungültige Auswahl.";
const TEAM_DELETE_SUCCESS: &str = "Team gelöscht.";
const TEAM_DELETE_NOT_FOUND: &str = "Team nicht gefunden – vermutlich schon gelöscht.";
const TEAM_DELETE_ERROR: &str = "Konnte das Team nicht löschen. Versuch es später nochmal.";

pub fn rank_display(rank_name: &str, subrank: i64) -> String {
    if rank_name.is_empty() {
        return "Unbekannt".to_string();
    }
    if subrank > 0 {
        format!("{rank_name} {subrank}")
    } else {
        rank_name.to_string()
    }
}

/// Anmeldung offen? (wie `_is_period_open`: aktiv + jetzt im Fenster,
/// lokale Zeit wie das Original).
pub fn is_period_open(period: Option<&Value>, now_local: chrono::NaiveDateTime) -> bool {
    let Some(period) = period else { return false };
    if period.get("is_active").and_then(Value::as_i64).unwrap_or(0) == 0 {
        return false;
    }
    let parse = |key: &str| {
        period
            .get(key)
            .and_then(Value::as_str)
            .and_then(crate::web::parse_period_dt)
    };
    match (parse("registration_start"), parse("registration_end")) {
        (Some(start), Some(end)) => start <= now_local && now_local <= end,
        _ => false,
    }
}

/// `_fmt_dt`: ISO-Datum → „TT.MM.JJJJ HH:MM Uhr", sonst „—".
fn fmt_dt(raw: Option<&str>) -> String {
    match raw.and_then(crate::web::parse_period_dt) {
        Some(dt) => dt.format("%d.%m.%Y %H:%M Uhr").to_string(),
        None => "—".to_string(),
    }
}

/// `_period_status_str`: Kein Zeitraum / Geschlossen / Startet … / Abgelaufen / Offen.
fn period_status_str(period: Option<&Value>, now: chrono::NaiveDateTime) -> String {
    let Some(p) = period else {
        return "Kein Zeitraum".to_string();
    };
    if p.get("is_active").and_then(Value::as_i64).unwrap_or(0) == 0 {
        return "⛔ Geschlossen".to_string();
    }
    let parse = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .and_then(crate::web::parse_period_dt)
    };
    match (parse("registration_start"), parse("registration_end")) {
        (Some(start), Some(end)) => {
            if now < start {
                format!(
                    "⏳ Startet {}",
                    fmt_dt(p.get("registration_start").and_then(Value::as_str))
                )
            } else if now > end {
                "⛔ Abgelaufen".to_string()
            } else {
                "🟢 Offen".to_string()
            }
        }
        _ => "Unbekannt".to_string(),
    }
}

/// Persistentes Anmelde-Panel (Port von `_build_panel_embed`, turnier.py:115-149).
fn panel_embed(
    period: Option<&Value>,
    summary: Option<&Value>,
    now: chrono::NaiveDateTime,
) -> Value {
    let active = period.filter(|p| p.get("is_active").and_then(Value::as_i64).unwrap_or(0) != 0);
    let mut fields: Vec<Value> = Vec::new();
    let mut description: Option<String> = None;

    if let Some(p) = active {
        fields.push(json!({
            "name": "📅 Zeitraum",
            "value": p.get("name").and_then(Value::as_str).unwrap_or("—"),
            "inline": false,
        }));
        fields.push(
            json!({ "name": "Status", "value": period_status_str(period, now), "inline": true }),
        );
        fields.push(json!({
            "name": "🕐 Start",
            "value": fmt_dt(p.get("registration_start").and_then(Value::as_str)),
            "inline": true,
        }));
        fields.push(json!({
            "name": "🕐 Ende",
            "value": fmt_dt(p.get("registration_end").and_then(Value::as_str)),
            "inline": true,
        }));
    } else {
        description = Some("Aktuell ist **kein Anmeldezeitraum** aktiv.".to_string());
    }

    if let Some(s) = summary {
        let g = |k: &str| s.get(k).and_then(Value::as_i64).unwrap_or(0);
        fields.push(json!({
            "name": "👥 Anmeldungen",
            "value": format!(
                "Gesamt: **{}** | Solo: **{}** | Team: **{}**",
                g("signups_total"), g("solo_count"), g("team_count")
            ),
            "inline": false,
        }));
    }

    fields.push(json!({
        "name": "ℹ️ Voraussetzungen",
        "value": format!(
            "• Du benötigst die <@&{TURNIER_ROLE_ID}> Rolle\n\
             • Steam-Konto verknüpfen: `/account_verknüpfen`"
        ),
        "inline": false,
    }));

    let mut embed = json!({
        "title": "🏆 Deadlock Turnier-Anmeldung",
        "color": 0xF1C40F,
        "fields": fields,
    });
    if let Some(desc) = description {
        embed["description"] = json!(desc);
    }
    embed
}

/// Persistente Panel-Buttons (Original-custom_ids `turnier_panel_*`).
fn panel_components() -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 3, "label": "Jetzt anmelden", "emoji": { "name": "✅" },
          "custom_id": "turnier_panel_anmelden" },
        { "type": 2, "style": 4, "label": "Abmelden", "emoji": { "name": "🚪" },
          "custom_id": "turnier_panel_abmelden" },
        { "type": 2, "style": 2, "label": "Mein Status", "emoji": { "name": "📊" },
          "custom_id": "turnier_panel_status" },
    ]}])
}

fn period_team_size(period: Option<&Value>) -> i64 {
    period
        .and_then(|p| p.get("team_size"))
        .and_then(Value::as_i64)
        .filter(|size| *size > 0)
        .unwrap_or(TEAM_MAX_SIZE)
}

fn mode_components(user_id: u64, rank_name: &str, rank_sub: i64, team_size: i64) -> Value {
    // Rang in die ID einbetten — das Menü ist zustandslos
    let rank = rank_name.to_lowercase();
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": "Solo anmelden", "emoji": { "name": "🎯" },
          "custom_id": format!("tn:solo:{rank}:{rank_sub}:{team_size}:{user_id}") },
        { "type": 2, "style": 2, "label": "Mit Team anmelden", "emoji": { "name": "🛡️" },
          "custom_id": format!("tn:team:{rank}:{rank_sub}:{team_size}:{user_id}") },
    ]}])
}

fn signup_success_embed(
    mode: &str,
    status: &str,
    rank_name: &str,
    rank_sub: i64,
    team: Option<&str>,
) -> Value {
    let status_txt = match status {
        "inserted" => "Eingetragen",
        "updated" => "Aktualisiert",
        _ => "Unverändert",
    };
    let title = if mode == "solo" {
        "✅ Solo-Anmeldung erfolgreich"
    } else {
        "✅ Team-Anmeldung erfolgreich"
    };
    let mut fields = vec![
        json!({ "name": "Status", "value": status_txt, "inline": true }),
        json!({ "name": "Modus", "value": if mode == "solo" { "Solo" } else { "Team" }, "inline": true }),
        json!({ "name": "Rang", "value": rank_display(rank_name, rank_sub), "inline": true }),
    ];
    if let Some(team) = team {
        fields.push(json!({ "name": "Team", "value": team, "inline": true }));
    }
    json!({ "title": title, "color": 0x2ECC71, "fields": fields })
}

fn truncate_chars(raw: &str, max: usize) -> String {
    raw.chars().take(max).collect()
}

fn admin_owner(custom_id: &str) -> Option<u64> {
    custom_id
        .split(':')
        .next_back()
        .and_then(|raw| raw.parse().ok())
}

fn admin_action(custom_id: &str) -> &str {
    custom_id.split(':').nth(1).unwrap_or_default()
}

fn parse_admin_page(custom_id: &str) -> usize {
    custom_id
        .split(':')
        .nth(2)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0)
}

fn max_signup_page(total: usize) -> usize {
    total.saturating_sub(1) / SIGNUP_PAGE_SIZE
}

fn clamp_signup_page(total: usize, page: usize) -> usize {
    page.min(max_signup_page(total))
}

fn admin_dashboard_embed(
    period: Option<&Value>,
    summary: Option<&Value>,
    now: chrono::NaiveDateTime,
) -> Value {
    let active = period.filter(|p| p.get("is_active").and_then(Value::as_i64).unwrap_or(0) != 0);
    let mut fields = Vec::new();
    let mut embed = json!({
        "title": ADMIN_DASH_TITLE,
        "color": 0xE74C3C,
    });

    if let Some(p) = active {
        fields.push(json!({
            "name": ADMIN_DASH_PERIOD_FIELD,
            "value": p.get("name").and_then(Value::as_str).unwrap_or("—"),
            "inline": false,
        }));
        fields.push(json!({
            "name": ADMIN_DASH_STATUS_FIELD,
            "value": period_status_str(period, now),
            "inline": true,
        }));
        fields.push(json!({
            "name": ADMIN_DASH_END_FIELD,
            "value": fmt_dt(p.get("registration_end").and_then(Value::as_str)),
            "inline": true,
        }));
    } else {
        embed["description"] = json!(ADMIN_DASH_NO_ACTIVE_PERIOD_DESC);
    }

    let g = |key: &str| {
        summary
            .and_then(|s| s.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    fields.push(json!({
        "name": ADMIN_DASH_SUMMARY_FIELD,
        "value": format!(
            "{ADMIN_DASH_SUMMARY_LABEL}: {} | {} | {} | {}",
            g("signups_total"),
            g("solo_count"),
            g("team_count"),
            g("teams_count")
        ),
        "inline": false,
    }));
    embed["fields"] = json!(fields);
    embed
}

fn admin_dashboard_components(owner: u64) -> Value {
    json!([
        { "type": 1, "components": [
            { "type": 2, "style": 1, "label": BTN_PERIOD_CREATE, "emoji": { "name": "📅" },
              "custom_id": format!("tnadm:period_create:{owner}") },
            { "type": 2, "style": 4, "label": BTN_PERIOD_CLOSE, "emoji": { "name": "🔴" },
              "custom_id": format!("tnadm:period_close:{owner}") },
            { "type": 2, "style": 2, "label": BTN_SIGNUPS_MANAGE, "emoji": { "name": "📋" },
              "custom_id": format!("tnadm:signups:0:{owner}") },
        ]},
        { "type": 1, "components": [
            { "type": 2, "style": 2, "label": BTN_TEAMS_MANAGE, "emoji": { "name": "🛡️" },
              "custom_id": format!("tnadm:teams:{owner}") },
            { "type": 2, "style": 3, "label": BTN_PANEL_POST, "emoji": { "name": "📢" },
              "custom_id": format!("tnadm:panel:{owner}") },
            { "type": 2, "style": 4, "label": BTN_CLEAR_OPEN, "emoji": { "name": "🗑️" },
              "custom_id": format!("tnadm:clear_open:{owner}") },
        ]},
    ])
}

fn dashboard_components(
    owner: u64,
    is_admin: bool,
    period_open: bool,
    is_signed_up: bool,
) -> Value {
    let mut rows = Vec::new();
    let mut user_buttons: Vec<Value> = Vec::new();
    if period_open && !is_signed_up {
        user_buttons.push(json!({ "type": 2, "style": 1, "label": "Anmelden", "custom_id": "turnier_panel_anmelden" }));
    }
    if is_signed_up {
        user_buttons.push(json!({ "type": 2, "style": 4, "label": "Abmelden", "custom_id": "turnier_panel_abmelden" }));
    }
    user_buttons.push(
        json!({ "type": 2, "style": 2, "label": "Status", "custom_id": "turnier_panel_status" }),
    );
    rows.push(json!({ "type": 1, "components": user_buttons }));
    if is_admin {
        rows.push(json!({ "type": 1, "components": [{
            "type": 2,
            "style": 1,
            "label": BTN_ADMIN_DASHBOARD,
            "emoji": { "name": "🛠️" },
            "custom_id": format!("tnadm:dashboard:{owner}"),
        }]}));
    }
    json!(rows)
}

fn period_create_modal(owner: u64) -> ModalSpec {
    ModalSpec {
        custom_id: format!("tnadm:period_submit:{owner}"),
        title: PERIOD_CREATE_MODAL_TITLE.to_string(),
        fields: vec![
            ModalField {
                custom_id: "period_name".to_string(),
                label: PERIOD_NAME_LABEL.to_string(),
                placeholder: PERIOD_NAME_PLACEHOLDER.to_string(),
                required: true,
                min_length: 3,
                max_length: 64,
                paragraph: false,
            },
            ModalField {
                custom_id: "start_dt".to_string(),
                label: PERIOD_START_LABEL.to_string(),
                placeholder: PERIOD_START_PLACEHOLDER.to_string(),
                required: true,
                min_length: 12,
                max_length: 16,
                paragraph: false,
            },
            ModalField {
                custom_id: "end_dt".to_string(),
                label: PERIOD_END_LABEL.to_string(),
                placeholder: PERIOD_END_PLACEHOLDER.to_string(),
                required: true,
                min_length: 12,
                max_length: 16,
                paragraph: false,
            },
            ModalField {
                custom_id: "team_size_input".to_string(),
                label: PERIOD_TEAM_SIZE_LABEL.to_string(),
                placeholder: PERIOD_TEAM_SIZE_PLACEHOLDER.to_string(),
                required: false,
                min_length: 1,
                max_length: 2,
                paragraph: false,
            },
        ],
    }
}

fn admin_team_create_modal(owner: u64) -> ModalSpec {
    ModalSpec {
        custom_id: format!("tnadm:team_submit:{owner}"),
        title: TEAM_CREATE_MODAL_TITLE.to_string(),
        fields: vec![ModalField {
            custom_id: "team_name".to_string(),
            label: TEAM_NAME_LABEL.to_string(),
            placeholder: TEAM_NAME_PLACEHOLDER.to_string(),
            required: true,
            min_length: store::TEAM_NAME_MIN as u16,
            max_length: store::TEAM_NAME_MAX as u16,
            paragraph: false,
        }],
    }
}

fn parse_period_submit(
    options: &std::collections::HashMap<String, Value>,
) -> Option<(String, String, String, i64)> {
    let name = options
        .get("period_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| (3..=64).contains(&name.chars().count()))?
        .to_string();
    let parse_dt = |key: &str| {
        options
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .and_then(|raw| chrono::NaiveDateTime::parse_from_str(raw, "%d.%m.%Y %H:%M").ok())
    };
    let start = parse_dt("start_dt")?;
    let end = parse_dt("end_dt")?;
    if end <= start {
        return None;
    }
    let team_size_raw = options
        .get("team_size_input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|raw| !raw.is_empty());
    let team_size = match team_size_raw {
        Some(raw) => raw.parse::<i64>().ok()?,
        None => TEAM_MAX_SIZE,
    };
    if !(2..=20).contains(&team_size) {
        return None;
    }
    Some((
        name,
        start.format("%Y-%m-%dT%H:%M:%S").to_string(),
        end.format("%Y-%m-%dT%H:%M:%S").to_string(),
        team_size,
    ))
}

fn signup_manage_components(
    owner: u64,
    page: usize,
    max_page: usize,
    options: Vec<Value>,
) -> Value {
    let mut rows = Vec::new();
    if !options.is_empty() {
        rows.push(json!({ "type": 1, "components": [{
            "type": 3,
            "custom_id": format!("tnadm:signup_remove:{page}:{owner}"),
            "placeholder": SIGNUP_REMOVE_SELECT_PLACEHOLDER,
            "options": options,
            "min_values": 1,
            "max_values": 1,
        }]}));
    }
    let prev = page.saturating_sub(1);
    let next = if page < max_page { page + 1 } else { page };
    rows.push(json!({ "type": 1, "components": [
        { "type": 2, "style": 2, "label": "◀", "custom_id": format!("tnadm:signups:{prev}:{owner}") },
        { "type": 2, "style": 2, "label": "▶", "custom_id": format!("tnadm:signups:{next}:{owner}") },
    ]}));
    json!(rows)
}

fn team_admin_components(owner: u64) -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": BTN_TEAM_CREATE, "emoji": { "name": "➕" },
          "custom_id": format!("tnadm:team_create:{owner}") },
        { "type": 2, "style": 4, "label": BTN_TEAM_DELETE_OPEN, "emoji": { "name": "🗑️" },
          "custom_id": format!("tnadm:team_delete_open:{owner}") },
    ]}])
}

fn clear_confirm_components(owner: u64) -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 4, "label": BTN_CLEAR_CONFIRM_YES,
          "custom_id": format!("tnadm:clear_yes:{owner}") },
        { "type": 2, "style": 2, "label": BTN_CLEAR_CONFIRM_NO,
          "custom_id": format!("tnadm:clear_no:{owner}") },
    ]}])
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait TurnierPort: Send + Sync {
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> Option<String>;
    async fn member_is_admin(&self, guild_id: u64, user_id: u64) -> bool;
}

pub struct TurnierUi {
    pub store: Arc<TournamentStore>,
    pub port: Arc<dyn TurnierPort>,
}

struct TurnierHandler {
    ui: Arc<TurnierUi>,
}

#[async_trait::async_trait]
impl InteractionHandler for TurnierHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        // /turnier-Slash → Dashboard-Embed.
        if interaction.command == "turnier" {
            return self.dashboard(interaction).await;
        }
        // /turnierpanel-Slash → Panel öffentlich in den Kanal posten (Admin).
        if interaction.command == "turnierpanel" {
            return self.post_panel(interaction).await;
        }
        if interaction.custom_id.starts_with("tnadm:") {
            return self.handle_admin(interaction).await;
        }
        let ui = &self.ui;
        match interaction.custom_id.as_str() {
            "turnier_panel_anmelden" => {
                let roles = ui
                    .port
                    .member_role_ids(interaction.guild_id, interaction.user_id)
                    .await;
                if !roles.contains(&TURNIER_ROLE_ID) {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Du benötigst die <@&{TURNIER_ROLE_ID}> Rolle, um dich anzumelden."
                    ));
                }
                let period = ui.store.active_period_json(interaction.guild_id).await;
                if !is_period_open(period.as_ref(), chrono::Local::now().naive_local()) {
                    return BridgeReply::ephemeral_text(
                        "❌ Die Anmeldung ist aktuell **nicht geöffnet**.",
                    );
                }
                let Some((rank_name, rank_sub)) =
                    ui.store.verified_steam_rank(interaction.user_id).await
                else {
                    return BridgeReply::ephemeral_text(
                        "❌ Dein Steam-Konto ist noch nicht mit Discord verknüpft.\n\
Nutze `/account_verknüpfen` auf dem Server, um dein Konto zu verbinden.",
                    );
                };
                let team_size = period_team_size(period.as_ref());
                BridgeReply {
                    embeds: vec![json!({
                        "title": "🏆 Turnier-Anmeldung",
                        "description": format!(
                            "Dein aktueller Rang: **{}**\nMax. Teamgröße: **{team_size}** Spieler\n\n\
                    Möchtest du dich **solo** oder **mit einem Team** anmelden?",
                            rank_display(&rank_name, rank_sub)
                        ),
                        "color": 0xF1C40F,
                    })],
                    components: Some(mode_components(
                        interaction.user_id,
                        &rank_name,
                        rank_sub,
                        team_size,
                    )),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "turnier_panel_abmelden" => {
                let removed = ui
                    .store
                    .remove_signup(interaction.guild_id, interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text(if removed {
                    "✅ Du wurdest aus dem Turnier abgemeldet."
                } else {
                    "ℹ️ Du warst nicht angemeldet."
                })
            }
            "turnier_panel_status" => {
                let Some(signup) = ui
                    .store
                    .get_signup(interaction.guild_id, interaction.user_id)
                    .await
                else {
                    return BridgeReply::ephemeral_text(
                        "ℹ️ Du bist aktuell **nicht** für das Turnier angemeldet.",
                    );
                };
                let rank_name = store::rank_label(&signup.rank);
                let mode = if signup.registration_mode == "team" {
                    "Team"
                } else {
                    "Solo"
                };
                BridgeReply {
                    embeds: vec![json!({
                        "title": "📊 Mein Turnierstatus",
                        "color": 0x3498DB,
                        "fields": [
                            { "name": "Rang", "value": rank_display(rank_name, signup.rank_subvalue), "inline": true },
                            { "name": "Modus", "value": mode, "inline": true },
                            { "name": "Team", "value": signup.team_name.unwrap_or_else(|| "—".to_string()), "inline": true },
                        ],
                    })],
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            _ => self.handle_flow(interaction).await,
        }
    }
}

impl TurnierHandler {
    /// `/turnier`-Dashboard (Port von `turnier_cmd`): Zeitraum- + Nutzer-Status
    /// plus Panel- und Admin-Buttons je nach Lage.
    async fn dashboard(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        let now = chrono::Local::now().naive_local();
        let period = ui.store.active_period_json(interaction.guild_id).await;
        let summary = ui.store.summary(interaction.guild_id).await.ok();
        let signup = ui
            .store
            .get_signup(interaction.guild_id, interaction.user_id)
            .await;
        let period_open = is_period_open(period.as_ref(), now);
        let is_admin = ui
            .port
            .member_is_admin(interaction.guild_id, interaction.user_id)
            .await;
        // Aktive Periode (is_active != 0) — None deckt „kein/inaktiv" ab.
        let active_period = period
            .as_ref()
            .filter(|p| p.get("is_active").and_then(Value::as_i64).unwrap_or(0) != 0);

        let mut fields: Vec<Value> = Vec::new();
        let mut description = String::new();

        if let Some(p) = active_period {
            fields.push(json!({
                "name": "📅 Zeitraum",
                "value": p.get("name").and_then(Value::as_str).unwrap_or("—"),
                "inline": false,
            }));
            fields.push(json!({
                "name": "Status",
                "value": period_status_str(period.as_ref(), now),
                "inline": true,
            }));
            fields.push(json!({
                "name": "🕐 Ende",
                "value": fmt_dt(p.get("registration_end").and_then(Value::as_str)),
                "inline": true,
            }));
            let g = |k: &str| {
                summary
                    .as_ref()
                    .and_then(|s| s.get(k))
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            };
            fields.push(json!({
                "name": "👥 Anmeldungen",
                "value": format!(
                    "**{}** gesamt  |  Solo: {}  |  Team: {}",
                    g("signups_total"), g("solo_count"), g("team_count")
                ),
                "inline": false,
            }));
        } else {
            description = "Kein aktiver Anmeldezeitraum.".to_string();
        }

        if let Some(s) = &signup {
            let rank_name = store::rank_label(&s.rank);
            let mode = if s.registration_mode == "team" {
                "Team"
            } else {
                "Solo"
            };
            let team = s.team_name.clone().unwrap_or_else(|| "—".to_string());
            fields.push(json!({
                "name": "✅ Du bist angemeldet",
                "value": format!(
                    "Rang: **{}**  |  Modus: {mode}  |  Team: {team}",
                    rank_display(rank_name, s.rank_subvalue)
                ),
                "inline": false,
            }));
        } else {
            let roles = ui
                .port
                .member_role_ids(interaction.guild_id, interaction.user_id)
                .await;
            if !roles.contains(&TURNIER_ROLE_ID) {
                fields.push(json!({
                    "name": "⚠️ Fehlende Rolle",
                    "value": format!("Du benötigst die <@&{TURNIER_ROLE_ID}> Rolle."),
                    "inline": false,
                }));
            } else if period_open {
                fields.push(json!({
                    "name": "📋 Status",
                    "value": "Noch nicht angemeldet. Klicke **Anmelden**.",
                    "inline": false,
                }));
            }
        }

        let mut embed =
            json!({ "title": "🏆 Deadlock Turnier", "color": 0xF1C40F, "fields": fields });
        if !description.is_empty() {
            if is_admin {
                description.push('\n');
                description.push_str(ADMIN_DASHBOARD_HINT);
            }
            embed["description"] = json!(description);
        }

        BridgeReply {
            embeds: vec![embed],
            components: Some(dashboard_components(
                interaction.user_id,
                is_admin,
                period_open,
                signup.is_some(),
            )),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    /// `/turnierpanel` (Port von `AdminDashboardView.post_panel_btn` /
    /// `balance_tournament_panel`): postet das persistente Anmelde-Panel
    /// (Embed + `turnier_panel_*`-Buttons) öffentlich in den aktuellen Kanal und
    /// bestätigt ephemeral. Admin-gated über `default_member_permissions` (Manage
    /// Guild) bei der Command-Registrierung — Discord blockt serverseitig.
    async fn post_panel(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        let now = chrono::Local::now().naive_local();
        let period = ui.store.active_period_json(interaction.guild_id).await;
        let summary = ui.store.summary(interaction.guild_id).await.ok();
        let embed = panel_embed(period.as_ref(), summary.as_ref(), now);
        BridgeReply {
            channel_message: Some(ChannelMessage {
                embeds: vec![embed],
                components: Some(panel_components()),
                confirmation: "✅ Panel in diesem Channel gepostet.".to_string(),
            }),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    async fn require_admin_owner(
        &self,
        interaction: &BridgeInteraction,
    ) -> Result<u64, BridgeReply> {
        let Some(owner) = admin_owner(&interaction.custom_id) else {
            return Err(BridgeReply::ephemeral_text(ADMIN_ERR_INVALID_CONTEXT));
        };
        if owner != interaction.user_id {
            return Err(BridgeReply::ephemeral_text(ADMIN_ERR_OWNER_ONLY));
        }
        if !self
            .ui
            .port
            .member_is_admin(interaction.guild_id, interaction.user_id)
            .await
        {
            return Err(BridgeReply::ephemeral_text(ADMIN_ERR_PERMISSION_REQUIRED));
        }
        Ok(owner)
    }

    async fn handle_admin(&self, interaction: BridgeInteraction) -> BridgeReply {
        let owner = match self.require_admin_owner(&interaction).await {
            Ok(owner) => owner,
            Err(reply) => return reply,
        };
        let action = admin_action(&interaction.custom_id).to_string();
        match action.as_str() {
            "dashboard" => {
                self.admin_dashboard_reply(interaction.guild_id, owner)
                    .await
            }
            "period_create" => BridgeReply {
                modal: Some(period_create_modal(owner)),
                ..BridgeReply::default()
            },
            "period_submit" => self.submit_period(interaction, owner).await,
            "period_close" => self.close_period(interaction.guild_id).await,
            "signups" => {
                let page = parse_admin_page(&interaction.custom_id);
                self.signup_manage_reply(interaction.guild_id, owner, page, false, None)
                    .await
            }
            "signup_remove" => {
                let page = parse_admin_page(&interaction.custom_id);
                let Some(user_id) = interaction
                    .values
                    .first()
                    .and_then(|raw| raw.parse::<u64>().ok())
                else {
                    return BridgeReply::ephemeral_text(SIGNUP_REMOVE_INVALID_SELECTION);
                };
                if self
                    .ui
                    .store
                    .remove_signup(interaction.guild_id, user_id)
                    .await
                {
                    self.signup_manage_reply(
                        interaction.guild_id,
                        owner,
                        page,
                        true,
                        Some(SIGNUP_REMOVE_SUCCESS.to_string()),
                    )
                    .await
                } else {
                    BridgeReply::ephemeral_text(SIGNUP_REMOVE_NOT_FOUND)
                }
            }
            "teams" => self.teams_reply(interaction.guild_id, owner).await,
            "team_create" => BridgeReply {
                modal: Some(admin_team_create_modal(owner)),
                ..BridgeReply::default()
            },
            "team_submit" => self.submit_team(interaction).await,
            "team_delete_open" => {
                self.team_delete_select_reply(interaction.guild_id, owner)
                    .await
            }
            "team_delete" => self.delete_team(interaction).await,
            "clear_open" => BridgeReply {
                content: Some(CLEAR_CONFIRM_PROMPT.to_string()),
                components: Some(clear_confirm_components(owner)),
                ephemeral: true,
                ..BridgeReply::default()
            },
            "clear_yes" => match self.ui.store.clear_all_signups(interaction.guild_id).await {
                Ok(_) => BridgeReply {
                    content: Some(CLEAR_SUCCESS.to_string()),
                    components: Some(json!([])),
                    update_message: true,
                    ..BridgeReply::default()
                },
                Err(err) => {
                    tracing::warn!(%err, guild_id = interaction.guild_id, "Turnier-Admin: clear_all_signups fehlgeschlagen");
                    BridgeReply::ephemeral_text(CLEAR_ERROR)
                }
            },
            "clear_no" => BridgeReply {
                content: Some(CLEAR_CANCELLED.to_string()),
                components: Some(json!([])),
                update_message: true,
                ..BridgeReply::default()
            },
            "panel" => self.post_panel(interaction).await,
            _ => BridgeReply::ephemeral_text(ADMIN_ERR_UNKNOWN_ACTION),
        }
    }

    async fn admin_dashboard_reply(&self, guild_id: u64, owner: u64) -> BridgeReply {
        let now = chrono::Local::now().naive_local();
        let period = self.ui.store.active_period_json(guild_id).await;
        let summary = self.ui.store.summary(guild_id).await.ok();
        BridgeReply {
            embeds: vec![admin_dashboard_embed(
                period.as_ref(),
                summary.as_ref(),
                now,
            )],
            components: Some(admin_dashboard_components(owner)),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    async fn submit_period(&self, interaction: BridgeInteraction, owner: u64) -> BridgeReply {
        let Some((name, start, end, team_size)) = parse_period_submit(&interaction.options) else {
            return BridgeReply::ephemeral_text(PERIOD_CREATE_INVALID_INPUT);
        };
        match self
            .ui
            .store
            .create_period(
                interaction.guild_id,
                &name,
                &start,
                &end,
                team_size,
                Some(owner),
            )
            .await
        {
            Ok(_) => BridgeReply::ephemeral_text(PERIOD_CREATE_SUCCESS),
            Err(err) => {
                tracing::warn!(%err, guild_id = interaction.guild_id, "Turnier-Admin: create_period fehlgeschlagen");
                BridgeReply::ephemeral_text(PERIOD_CREATE_ERROR)
            }
        }
    }

    async fn close_period(&self, guild_id: u64) -> BridgeReply {
        let Some(period) = self.ui.store.active_period_json(guild_id).await else {
            return BridgeReply::ephemeral_text(PERIOD_CLOSE_NO_ACTIVE_PERIOD);
        };
        let Some(period_id) = period.get("id").and_then(Value::as_i64) else {
            return BridgeReply::ephemeral_text(PERIOD_CLOSE_INVALID_ACTIVE_PERIOD);
        };
        match self.ui.store.close_period(guild_id, period_id).await {
            Ok(_) => BridgeReply::ephemeral_text(PERIOD_CLOSE_SUCCESS),
            Err(err) => {
                tracing::warn!(%err, guild_id, period_id, "Turnier-Admin: close_period fehlgeschlagen");
                BridgeReply::ephemeral_text(PERIOD_CLOSE_ERROR)
            }
        }
    }

    async fn signup_manage_reply(
        &self,
        guild_id: u64,
        owner: u64,
        page: usize,
        update_message: bool,
        content: Option<String>,
    ) -> BridgeReply {
        let signups = self.ui.store.list_signups(guild_id).await;
        if signups.is_empty() && !update_message {
            return BridgeReply::ephemeral_text(SIGNUPS_EMPTY_REPLY);
        }
        let page = clamp_signup_page(signups.len(), page);
        let max_page = max_signup_page(signups.len());
        let start = page * SIGNUP_PAGE_SIZE;
        let page_rows = signups
            .iter()
            .enumerate()
            .skip(start)
            .take(SIGNUP_PAGE_SIZE);
        let mut rows = Vec::new();
        for (idx, signup) in page_rows {
            let display = self
                .ui
                .port
                .member_display_name(guild_id, signup.user_id)
                .await
                .or_else(|| signup.display_name.clone())
                .unwrap_or_else(|| signup.user_id.to_string());
            rows.push((idx + 1, signup.clone(), display));
        }

        let mut lines = Vec::new();
        let mut options = Vec::new();
        for (idx, signup, display) in &rows {
            let mut rank = store::rank_label(&signup.rank).to_string();
            if signup.rank_subvalue > 0 {
                rank.push_str(&format!(" {}", signup.rank_subvalue));
            }
            let mode = if signup.registration_mode == "team" {
                "Team"
            } else {
                "Solo"
            };
            let team = signup.team_name.as_deref().unwrap_or("—");
            lines.push(format!("{idx}. **{display}** | {rank} | {mode} | {team}"));
            let description = format!("{rank} | {team}");
            options.push(json!({
                "label": truncate_chars(display, 100),
                "value": signup.user_id.to_string(),
                "description": truncate_chars(&description, 100),
            }));
        }

        let embed = json!({
            "title": format!("{SIGNUPS_LIST_TITLE} ({})", signups.len()),
            "color": 0x1ABC9C,
            "description": if lines.is_empty() { SIGNUPS_LIST_EMPTY_DESC.to_string() } else { lines.join("\n") },
            "footer": { "text": format!("{SIGNUPS_LIST_FOOTER_LABEL} {}/{}", page + 1, max_page + 1) },
        });

        BridgeReply {
            content,
            embeds: vec![embed],
            components: Some(signup_manage_components(owner, page, max_page, options)),
            ephemeral: !update_message,
            update_message,
            ..BridgeReply::default()
        }
    }

    async fn teams_reply(&self, guild_id: u64, owner: u64) -> BridgeReply {
        let teams = self.ui.store.list_teams(guild_id).await;
        let mut embed = json!({
            "title": format!("{TEAMS_LIST_TITLE} ({})", teams.len()),
            "color": 0x3498DB,
        });
        if teams.is_empty() {
            embed["description"] = json!(TEAMS_EMPTY_DESC);
        } else {
            let lines: Vec<String> = teams
                .iter()
                .take(20)
                .map(|team| {
                    format!(
                        "• **{}** — {}/{} (ID: `{}`)",
                        team.name, team.member_count, TEAM_MAX_SIZE, team.id
                    )
                })
                .collect();
            embed["description"] = json!(lines.join("\n"));
            if teams.len() > 20 {
                embed["footer"] = json!({ "text": TEAMS_LIST_TRUNCATED_FOOTER });
            }
        }
        BridgeReply {
            embeds: vec![embed],
            components: Some(team_admin_components(owner)),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    async fn submit_team(&self, interaction: BridgeInteraction) -> BridgeReply {
        let name = interaction
            .options
            .get("team_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        match self
            .ui
            .store
            .get_or_create_team(interaction.guild_id, &name, Some(interaction.user_id))
            .await
        {
            Ok(_) => BridgeReply::ephemeral_text(TEAM_CREATE_SUCCESS),
            Err(err) => {
                tracing::warn!(%err, guild_id = interaction.guild_id, "Turnier-Admin: get_or_create_team fehlgeschlagen");
                BridgeReply::ephemeral_text(TEAM_CREATE_ERROR)
            }
        }
    }

    async fn team_delete_select_reply(&self, guild_id: u64, owner: u64) -> BridgeReply {
        let teams = self.ui.store.list_teams(guild_id).await;
        if teams.is_empty() {
            return BridgeReply::ephemeral_text(TEAM_DELETE_NO_TEAMS);
        }
        let options: Vec<Value> = teams
            .iter()
            .take(25)
            .map(|team| {
                json!({
                    "label": truncate_chars(&team.name, 100),
                    "value": team.id.to_string(),
                    "description": truncate_chars(
                        &format!("{TEAM_DELETE_OPTION_META_LABEL}: {} | {}", team.id, team.member_count),
                        100
                    ),
                })
            })
            .collect();
        BridgeReply {
            content: Some(TEAM_DELETE_SELECT_PROMPT.to_string()),
            components: Some(json!([{ "type": 1, "components": [{
                "type": 3,
                "custom_id": format!("tnadm:team_delete:{owner}"),
                "placeholder": TEAM_DELETE_SELECT_PLACEHOLDER,
                "options": options,
                "min_values": 1,
                "max_values": 1,
            }]}])),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    async fn delete_team(&self, interaction: BridgeInteraction) -> BridgeReply {
        let Some(team_id) = interaction
            .values
            .first()
            .and_then(|raw| raw.parse::<i64>().ok())
        else {
            return BridgeReply::ephemeral_text(TEAM_DELETE_INVALID_SELECTION);
        };
        match self
            .ui
            .store
            .delete_team(interaction.guild_id, team_id)
            .await
        {
            Ok(true) => BridgeReply {
                content: Some(TEAM_DELETE_SUCCESS.to_string()),
                components: Some(json!([])),
                update_message: true,
                ..BridgeReply::default()
            },
            Ok(false) => BridgeReply::ephemeral_text(TEAM_DELETE_NOT_FOUND),
            Err(err) => {
                tracing::warn!(%err, guild_id = interaction.guild_id, team_id, "Turnier-Admin: delete_team fehlgeschlagen");
                BridgeReply::ephemeral_text(TEAM_DELETE_ERROR)
            }
        }
    }

    /// tn:solo/team/pick/create-Flow (Besitzer steckt am ID-Ende).
    async fn handle_flow(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        let parts: Vec<&str> = interaction.custom_id.split(':').collect();
        let owner: u64 = parts
            .last()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or_default();
        if owner != interaction.user_id {
            return BridgeReply::ephemeral_text("Dieses Menü gehört dir nicht.");
        }
        let action = parts.get(1).copied().unwrap_or_default();
        let rank = parts.get(2).copied().unwrap_or("initiate").to_string();
        let rank_sub: i64 = parts.get(3).and_then(|r| r.parse().ok()).unwrap_or(0);
        let team_size = if parts.len() >= 6 {
            parts
                .get(4)
                .and_then(|raw| raw.parse::<i64>().ok())
                .filter(|size| *size > 0)
                .unwrap_or(TEAM_MAX_SIZE)
        } else {
            ui.store
                .active_period(interaction.guild_id)
                .await
                .map(|(_, _, size)| if size > 0 { size } else { TEAM_MAX_SIZE })
                .unwrap_or(TEAM_MAX_SIZE)
        };
        let rank_name = store::rank_label(&rank);

        match action {
            "solo" => {
                let result = ui
                    .store
                    .upsert_signup(
                        interaction.guild_id,
                        interaction.user_id,
                        "solo",
                        &rank,
                        rank_sub,
                        None,
                        false,
                        None,
                    )
                    .await;
                match result {
                    Ok(signup) => BridgeReply {
                        embeds: vec![signup_success_embed(
                            "solo",
                            &signup.status,
                            rank_name,
                            rank_sub,
                            None,
                        )],
                        ephemeral: true,
                        ..BridgeReply::default()
                    },
                    Err(err) => BridgeReply::ephemeral_text(format!("❌ {err}")),
                }
            }
            "team" => {
                let teams = ui.store.list_teams(interaction.guild_id).await;
                if teams.is_empty() {
                    return BridgeReply {
                        modal: Some(team_create_modal(
                            &rank,
                            rank_sub,
                            team_size,
                            interaction.user_id,
                        )),
                        ..BridgeReply::default()
                    };
                }
                let mut options: Vec<Value> = teams
                    .iter()
                    .take(24)
                    .map(|team| {
                        let full_tag = if team.member_count >= team_size {
                            " ✗ voll".to_string()
                        } else {
                            format!(" ({}/{team_size})", team.member_count)
                        };
                        json!({
                            "label": team.name.chars().take(100).collect::<String>(),
                            "value": team.id.to_string(),
                            "description": format!("Mitglieder: {}/{TEAM_MAX_SIZE}{full_tag}", team.member_count),
                        })
                    })
                    .collect();
                options.push(json!({
                    "label": "Neues Team erstellen",
                    "value": "__create__",
                    "emoji": { "name": "➕" },
                    "description": "Erstelle ein neues Team für dich",
                }));
                BridgeReply {
                    embeds: vec![json!({
                        "title": "🛡️ Team auswählen",
                        "description": "Wähle dein Team oder erstelle ein neues.",
                        "color": 0x3498DB,
                    })],
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 3,
                        "custom_id": format!("tn:pick:{rank}:{rank_sub}:{team_size}:{}", interaction.user_id),
                        "placeholder": "Team auswählen…",
                        "options": options,
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "pick" => {
                let Some(value) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                if value == "__create__" {
                    return BridgeReply {
                        modal: Some(team_create_modal(
                            &rank,
                            rank_sub,
                            team_size,
                            interaction.user_id,
                        )),
                        ..BridgeReply::default()
                    };
                }
                let team_id: i64 = value.parse().unwrap_or(0);
                if let Some(team) = ui.store.get_team(interaction.guild_id, team_id).await {
                    if team.member_count >= team_size {
                        return BridgeReply::ephemeral_text(format!(
                            "❌ Dieses Team ist bereits voll ({team_size}/{team_size})."
                        ));
                    }
                }
                self.join_team(&interaction, &rank, rank_sub, team_id).await
            }
            "create" => {
                let name = interaction
                    .options
                    .get("team_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let team = match ui
                    .store
                    .get_or_create_team(interaction.guild_id, &name, Some(interaction.user_id))
                    .await
                {
                    Ok(team) => team,
                    Err(err) => return BridgeReply::ephemeral_text(format!("❌ {err}")),
                };
                if !team.created && team.member_count >= team_size {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Team **{}** ist bereits voll ({}/{team_size}).",
                        team.name, team.member_count
                    ));
                }
                self.join_team(&interaction, &rank, rank_sub, team.id).await
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }

    async fn join_team(
        &self,
        interaction: &BridgeInteraction,
        rank: &str,
        rank_sub: i64,
        team_id: i64,
    ) -> BridgeReply {
        let result = self
            .ui
            .store
            .upsert_signup(
                interaction.guild_id,
                interaction.user_id,
                "team",
                rank,
                rank_sub,
                Some(team_id),
                false,
                None,
            )
            .await;
        match result {
            Ok(signup) => BridgeReply {
                embeds: vec![signup_success_embed(
                    "team",
                    &signup.status,
                    store::rank_label(rank),
                    rank_sub,
                    signup.team_name.as_deref(),
                )],
                ephemeral: true,
                ..BridgeReply::default()
            },
            Err(err) => BridgeReply::ephemeral_text(format!("❌ {err}")),
        }
    }
}

fn team_create_modal(rank: &str, rank_sub: i64, team_size: i64, user_id: u64) -> ModalSpec {
    ModalSpec {
        custom_id: format!("tn:create:{rank}:{rank_sub}:{team_size}:{user_id}"),
        title: "Neues Team erstellen".to_string(),
        fields: vec![ModalField {
            custom_id: "team_name".to_string(),
            label: "Teamname".to_string(),
            placeholder: "z. B. Team Alpha".to_string(),
            required: true,
            min_length: 2,
            max_length: 32,
            paragraph: false,
        }],
    }
}

pub fn register(router: &mut InteractionRouter, ui: Arc<TurnierUi>) {
    let handler = Arc::new(TurnierHandler { ui });
    router.on_command(
        "turnier",
        CommandSpec {
            definition: json!({
                "name": "turnier",
                "description": "Turnier-Dashboard: Anmelden, Status und Verwaltung",
                "type": 1,
                "dm_permission": false,
            }),
        },
        handler.clone(),
    );
    // Admin: Anmelde-Panel in den Kanal posten (Manage Guild = 32).
    router.on_command(
        "turnierpanel",
        CommandSpec {
            definition: json!({
                "name": "turnierpanel",
                "description": "Postet das Turnier-Anmelde-Panel in diesen Kanal (Admin).",
                "type": 1,
                "dm_permission": false,
                "default_member_permissions": "32",
            }),
        },
        handler.clone(),
    );
    router.on_custom_id("turnier_panel_anmelden", handler.clone());
    router.on_custom_id("turnier_panel_abmelden", handler.clone());
    router.on_custom_id("turnier_panel_status", handler.clone());
    router.on_prefix("tnadm:", handler.clone());
    router.on_prefix("tn:", handler);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTurnierPort;

    #[async_trait::async_trait]
    impl TurnierPort for MockTurnierPort {
        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            vec![TURNIER_ROLE_ID]
        }

        async fn member_display_name(&self, _guild_id: u64, user_id: u64) -> Option<String> {
            Some(format!("User {user_id}"))
        }

        async fn member_is_admin(&self, _guild_id: u64, _user_id: u64) -> bool {
            true
        }
    }

    async fn test_handler() -> (tempfile::TempDir, TurnierHandler) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_db::Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = TournamentStore::new(db);
        store.ensure_schema().await.expect("schema");
        let ui = Arc::new(TurnierUi {
            store: Arc::new(store),
            port: Arc::new(MockTurnierPort),
        });
        (dir, TurnierHandler { ui })
    }

    #[test]
    fn periode_offen() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 6, 10)
            .expect("datum")
            .and_hms_opt(12, 0, 0)
            .expect("uhrzeit");
        let period = json!({
            "is_active": 1,
            "registration_start": "2026-06-09T00:00:00",
            "registration_end": "2026-06-11T23:59:59",
        });
        assert!(is_period_open(Some(&period), now));
        let closed = json!({
            "is_active": 0,
            "registration_start": "2026-06-09T00:00:00",
            "registration_end": "2026-06-11T23:59:59",
        });
        assert!(!is_period_open(Some(&closed), now));
        let past = json!({
            "is_active": 1,
            "registration_start": "2026-06-01T00:00:00",
            "registration_end": "2026-06-02T00:00:00",
        });
        assert!(!is_period_open(Some(&past), now));
        assert!(!is_period_open(None, now));
    }

    #[test]
    fn anzeige_und_ids() {
        assert_eq!(TEAM_MAX_SIZE, 6);
        assert_eq!(rank_display("Archon", 3), "Archon 3");
        assert_eq!(rank_display("Archon", 0), "Archon");
        assert_eq!(rank_display("", 2), "Unbekannt");
        let components = mode_components(42, "Phantom", 2, 4);
        assert_eq!(
            components[0]["components"][0]["custom_id"],
            "tn:solo:phantom:2:4:42"
        );
        assert_eq!(
            components[0]["components"][1]["custom_id"],
            "tn:team:phantom:2:4:42"
        );
        let modal = team_create_modal("phantom", 2, 4, 42);
        assert_eq!(modal.custom_id, "tn:create:phantom:2:4:42");
        let embed = signup_success_embed("solo", "inserted", "Phantom", 2, None);
        assert_eq!(embed["fields"][0]["value"], "Eingetragen");
        let embed = signup_success_embed("team", "updated", "Archon", 0, Some("Alpha"));
        assert_eq!(embed["fields"].as_array().expect("fields").len(), 4);
    }

    #[test]
    fn admin_components_und_modals_tragen_owner() {
        let dashboard = dashboard_components(42, true, true, false);
        assert_eq!(
            dashboard[1]["components"][0]["custom_id"],
            "tnadm:dashboard:42"
        );

        let admin = admin_dashboard_components(42);
        assert_eq!(
            admin[0]["components"][0]["custom_id"],
            "tnadm:period_create:42"
        );
        assert_eq!(
            admin[0]["components"][1]["custom_id"],
            "tnadm:period_close:42"
        );
        assert_eq!(admin[0]["components"][2]["custom_id"], "tnadm:signups:0:42");
        assert_eq!(period_create_modal(42).custom_id, "tnadm:period_submit:42");
        assert_eq!(
            admin_team_create_modal(42).custom_id,
            "tnadm:team_submit:42"
        );

        let paging = signup_manage_components(
            42,
            1,
            2,
            vec![json!({
                "label": "A",
                "value": "100",
            })],
        );
        assert_eq!(
            paging[0]["components"][0]["custom_id"],
            "tnadm:signup_remove:1:42"
        );
        assert_eq!(
            paging[1]["components"][0]["custom_id"],
            "tnadm:signups:0:42"
        );
        assert_eq!(
            paging[1]["components"][1]["custom_id"],
            "tnadm:signups:2:42"
        );
    }

    #[tokio::test]
    async fn legacy_fuenfteilige_tn_id_nutzt_owner_und_team_size_fallback() {
        let (_dir, handler) = test_handler().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tn:team:phantom:2:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;

        let modal = reply.modal.expect("modal");
        assert_eq!(modal.custom_id, "tn:create:phantom:2:6:42");
        assert!(reply.content.is_none());
    }

    #[tokio::test]
    async fn admin_period_submit_und_close_nutzt_store() {
        let (_dir, handler) = test_handler().await;
        let modal = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:period_create:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await
            .modal
            .expect("modal");
        assert_eq!(modal.custom_id, "tnadm:period_submit:42");

        let mut options = std::collections::HashMap::new();
        options.insert("period_name".to_string(), json!("Cup #1"));
        options.insert("start_dt".to_string(), json!("01.07.2026 12:00"));
        options.insert("end_dt".to_string(), json!("02.07.2026 12:00"));
        options.insert("team_size_input".to_string(), json!("4"));
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:period_submit:42".to_string(),
                user_id: 42,
                guild_id: 1,
                options,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(PERIOD_CREATE_SUCCESS));
        let period = handler
            .ui
            .store
            .active_period_json(1)
            .await
            .expect("period");
        assert_eq!(period["team_size"], 4);
        assert_eq!(period["created_by"], 42);

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:period_close:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(PERIOD_CLOSE_SUCCESS));
        assert!(handler.ui.store.active_period_json(1).await.is_none());
    }

    #[tokio::test]
    async fn admin_signup_paging_remove_und_clear() {
        let (_dir, handler) = test_handler().await;
        for uid in 100..111 {
            handler
                .ui
                .store
                .upsert_signup(1, uid, "solo", "phantom", 0, None, false, None)
                .await
                .expect("signup");
        }

        let first_page = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:signups:0:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        let options = first_page.components.as_ref().expect("components")[0]["components"][0]
            ["options"]
            .as_array()
            .expect("options");
        assert_eq!(options.len(), 10);
        assert_eq!(
            first_page.components.as_ref().expect("components")[1]["components"][1]["custom_id"],
            "tnadm:signups:1:42"
        );

        let second_page = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:signups:1:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        let remove_value = second_page.components.as_ref().expect("components")[0]["components"][0]
            ["options"][0]["value"]
            .as_str()
            .expect("value")
            .to_string();
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:signup_remove:1:42".to_string(),
                user_id: 42,
                guild_id: 1,
                values: vec![remove_value],
                ..BridgeInteraction::default()
            })
            .await;
        assert!(reply.update_message);
        assert_eq!(
            handler.ui.store.summary(1).await.expect("summary")["signups_total"],
            10
        );

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:clear_yes:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        assert!(reply.update_message);
        assert_eq!(
            handler.ui.store.summary(1).await.expect("summary")["signups_total"],
            0
        );
    }

    #[tokio::test]
    async fn admin_team_create_delete_nutzt_store() {
        let (_dir, handler) = test_handler().await;
        let mut options = std::collections::HashMap::new();
        options.insert("team_name".to_string(), json!("Alpha"));
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:team_submit:42".to_string(),
                user_id: 42,
                guild_id: 1,
                options,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(TEAM_CREATE_SUCCESS));
        let teams = handler.ui.store.list_teams(1).await;
        assert_eq!(teams.len(), 1);

        let select = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:team_delete_open:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        let team_id = select.components.as_ref().expect("components")[0]["components"][0]
            ["options"][0]["value"]
            .as_str()
            .expect("team id")
            .to_string();
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:team_delete:42".to_string(),
                user_id: 42,
                guild_id: 1,
                values: vec![team_id],
                ..BridgeInteraction::default()
            })
            .await;
        assert!(reply.update_message);
        assert!(handler.ui.store.list_teams(1).await.is_empty());
    }

    #[tokio::test]
    async fn admin_delete_team_fremde_guild_team_id_meldet_not_found() {
        let (_dir, handler) = test_handler().await;
        let team = handler
            .ui
            .store
            .get_or_create_team(2, "Alpha", None)
            .await
            .expect("team");

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tnadm:team_delete:42".to_string(),
                user_id: 42,
                guild_id: 1,
                values: vec![team.id.to_string()],
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(reply.content.as_deref(), Some(TEAM_DELETE_NOT_FOUND));
        assert!(reply.ephemeral);
        assert!(!reply.update_message);
        assert!(reply.components.is_none());
        assert_eq!(handler.ui.store.list_teams(2).await.len(), 1);
    }

    #[test]
    fn panel_embed_und_buttons() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 6, 10)
            .expect("datum")
            .and_hms_opt(12, 0, 0)
            .expect("uhrzeit");
        // Aktive Periode → Zeitraum-Felder + Anmeldungen + Voraussetzungen.
        let period = json!({
            "is_active": 1,
            "name": "Cup #1",
            "registration_start": "2026-06-09T00:00:00",
            "registration_end": "2026-06-11T23:59:59",
        });
        let summary = json!({ "signups_total": 5, "solo_count": 2, "team_count": 3 });
        let embed = panel_embed(Some(&period), Some(&summary), now);
        assert_eq!(embed["title"], "🏆 Deadlock Turnier-Anmeldung");
        let fields = embed["fields"].as_array().expect("fields");
        // Zeitraum, Status, Start, Ende, Anmeldungen, Voraussetzungen = 6.
        assert_eq!(fields.len(), 6);
        assert!(embed.get("description").is_none());

        // Ohne aktive Periode → Beschreibung statt Zeitraum-Feldern.
        let embed = panel_embed(None, None, now);
        assert_eq!(
            embed["description"].as_str().expect("desc"),
            "Aktuell ist **kein Anmeldezeitraum** aktiv."
        );

        // Buttons tragen die Original-custom_ids.
        let comps = panel_components();
        let ids: Vec<&str> = comps[0]["components"]
            .as_array()
            .expect("buttons")
            .iter()
            .map(|b| b["custom_id"].as_str().expect("id"))
            .collect();
        assert_eq!(
            ids,
            vec![
                "turnier_panel_anmelden",
                "turnier_panel_abmelden",
                "turnier_panel_status"
            ]
        );
    }
}
