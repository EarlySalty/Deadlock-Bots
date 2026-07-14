use std::collections::{BTreeMap, HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Serialize, Serializer};
use serde_json::Value;
use sqlx::FromRow;

use crate::web::{err_text, ok_json, DashboardApp};

pub const NOISE_ACTION_TYPES: &[i32] = &[10, 11, 12, 13, 14, 15, 25, 26, 110, 111, 112];

const MODERATION_TYPES: &[i32] = &[20, 22, 23, 24, 27, 72, 73, 74, 75, 143, 144, 145];
const ROLE_TYPES: &[i32] = &[25, 30, 31, 32];
const STRUCTURE_TYPES: &[i32] = &[
    1, 10, 11, 12, 13, 14, 15, 40, 41, 42, 50, 51, 52, 60, 61, 62, 80, 81, 82, 110, 111, 112, 163,
    164, 165, 166, 167, 190, 191, 192, 193,
];

pub const CATEGORY_ACTION_TYPES: &[(&str, &[i32])] = &[
    ("moderation", MODERATION_TYPES),
    ("rollen", ROLE_TYPES),
    ("struktur", STRUCTURE_TYPES),
];

pub fn is_noise(actor_id: Option<i64>, action_type: i32, bot_user_id: i64) -> bool {
    actor_id == Some(bot_user_id) && NOISE_ACTION_TYPES.contains(&action_type)
}

pub fn category_for(action_type: i32) -> &'static str {
    CATEGORY_ACTION_TYPES
        .iter()
        .find_map(|(category, types)| types.contains(&action_type).then_some(*category))
        .unwrap_or("sonstiges")
}

#[derive(Debug, Clone, Copy)]
struct Snowflake(i64);

impl Serialize for Snowflake {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

#[derive(Debug)]
struct AuditQuery {
    from: NaiveDate,
    to: NaiveDate,
    categories: Vec<String>,
    action_type: Option<i32>,
    actor: Option<i64>,
    target: Option<i64>,
    search: Option<String>,
    hide_noise: bool,
    limit: usize,
    offset: usize,
}

#[derive(Debug, FromRow)]
struct AuditRow {
    entry_id: i64,
    action_type: i32,
    user_id: Option<i64>,
    target_id: Option<i64>,
    changes: Option<Value>,
    options: Option<Value>,
    reason: Option<String>,
    occurred_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct NamedId {
    id: Option<Snowflake>,
    name: Option<String>,
}

#[derive(Serialize)]
struct AuditEntry {
    entry_id: Snowflake,
    occurred_at: String,
    action_type: i32,
    action_name: String,
    category: &'static str,
    actor: NamedId,
    target: NamedId,
    changes: Value,
    options: Value,
    reason: Option<String>,
    is_noise: bool,
}

#[derive(Serialize)]
struct AuditResponse {
    entries: Vec<AuditEntry>,
    total: usize,
    counts: BTreeMap<&'static str, usize>,
    noise_hidden: usize,
}

fn parse_params(params: HashMap<String, String>, today: NaiveDate) -> Result<AuditQuery, Response> {
    let date = |key: &str| -> Result<Option<NaiveDate>, Response> {
        params
            .get(key)
            .map(|raw| {
                NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
                    .map_err(|_| err_text(400, &format!("{key} must be an ISO date")))
            })
            .transpose()
    };
    let from = date("from")?.unwrap_or(today - Duration::days(29));
    let to = date("to")?
        .unwrap_or(today)
        .checked_add_signed(Duration::days(1))
        .ok_or_else(|| err_text(400, "to date is out of range"))?;
    if from >= to {
        return Err(err_text(400, "from must be before to"));
    }

    let categories = params
        .get("category")
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if categories.iter().any(|value| {
        !matches!(
            value.as_str(),
            "moderation" | "rollen" | "struktur" | "sonstiges"
        )
    }) {
        return Err(err_text(400, "unknown category"));
    }

    let integer = |key: &str| -> Result<Option<i64>, Response> {
        params
            .get(key)
            .map(|raw| {
                raw.trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| err_text(400, &format!("{key} must be a positive integer")))
            })
            .transpose()
    };
    let action_type = integer("action_type")?
        .map(|value| i32::try_from(value).map_err(|_| err_text(400, "action_type is out of range")))
        .transpose()?;
    let actor = integer("actor")?;
    let target = integer("target")?;
    let hide_noise = match params.get("noise").map(|value| value.trim()) {
        None | Some("") | Some("hide") => true,
        Some("show") => false,
        Some(_) => return Err(err_text(400, "noise must be hide or show")),
    };
    let limit = integer("limit")?.unwrap_or(100).min(500) as usize;
    let offset = params
        .get("offset")
        .map(|raw| {
            raw.trim()
                .parse::<usize>()
                .map_err(|_| err_text(400, "offset must be a non-negative integer"))
        })
        .transpose()?
        .unwrap_or(0);

    Ok(AuditQuery {
        from,
        to,
        categories,
        action_type,
        actor,
        target,
        search: params
            .get("q")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase),
        hide_noise,
        limit,
        offset,
    })
}

fn action_name(action_type: i32) -> String {
    let name = match action_type {
        1 => "Server geändert",
        10 => "Kanal erstellt",
        11 => "Kanal geändert",
        12 => "Kanal gelöscht",
        13 => "Kanal-Berechtigung erstellt",
        14 => "Kanal-Berechtigung geändert",
        15 => "Kanal-Berechtigung gelöscht",
        20 => "Mitglied gekickt",
        22 => "Mitglied gebannt",
        23 => "Bann aufgehoben",
        24 => "Mitglied geändert (Mute/Timeout/Nick)",
        25 => "Rollen geändert",
        26 => "Mitglied verschoben",
        27 => "Mitglied getrennt",
        30 => "Rolle erstellt",
        31 => "Rolle geändert",
        32 => "Rolle gelöscht",
        40 => "Webhook erstellt",
        41 => "Webhook geändert",
        42 => "Webhook gelöscht",
        50 => "Emoji erstellt",
        51 => "Emoji geändert",
        52 => "Emoji gelöscht",
        60 => "Integration erstellt",
        61 => "Integration geändert",
        62 => "Integration gelöscht",
        72 => "Nachricht gelöscht",
        73 => "Nachrichten gesammelt gelöscht",
        74 => "Nachricht angepinnt",
        75 => "Pin entfernt",
        80 => "Sticker erstellt",
        81 => "Sticker geändert",
        82 => "Sticker gelöscht",
        110 => "Thread erstellt",
        111 => "Thread geändert",
        112 => "Thread gelöscht",
        143 => "AutoMod-Aktion ausgelöst",
        144 => "AutoMod-Meldung ausgelöst",
        145 => "AutoMod-Timeout ausgelöst",
        163 => "Onboarding-Prompt erstellt",
        164 => "Onboarding-Prompt geändert",
        165 => "Onboarding-Prompt gelöscht",
        166 => "Onboarding erstellt",
        167 => "Onboarding geändert",
        190 => "Startseiten-Funktion erstellt",
        191 => "Startseiten-Funktion geändert",
        192 => "Startseiten-Funktion gelöscht",
        193 => "Startseiten-Einstellungen geändert",
        _ => return format!("Unbekannt ({action_type})"),
    };
    name.to_string()
}

pub async fn audit_log(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let query = match parse_params(params, Utc::now().date_naive()) {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let bot_user_id = match i64::try_from(app.audit_bot_user_id()) {
        Ok(id) => id,
        Err(_) => return err_text(500, "DISCORD_BOT_USER_ID is out of range"),
    };

    // ponytail: Bei mehr als 1 Mio. Zeilen einen occurred_at-Index ergänzen; bei ~26k reicht der Seq-Scan.
    let rows = match sqlx::query_as::<_, AuditRow>(
        r#"
        SELECT entry_id, action_type, user_id, target_id, changes, options, reason, occurred_at
          FROM core.discord_audit_log
         WHERE occurred_at >= $1::date
           AND occurred_at < $2::date
         ORDER BY occurred_at DESC, entry_id DESC
        "#,
    )
    .bind(query.from)
    .bind(query.to)
    .fetch_all(app.pool())
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(%err, "Audit-Log konnte nicht geladen werden");
            return err_text(500, "Audit log unavailable");
        }
    };

    let noise_count = rows
        .iter()
        .filter(|row| is_noise(row.user_id, row.action_type, bot_user_id))
        .count();
    let filtered = rows
        .into_iter()
        .filter(|row| {
            query
                .action_type
                .is_none_or(|value| row.action_type == value)
        })
        .filter(|row| query.actor.is_none_or(|value| row.user_id == Some(value)))
        .filter(|row| {
            query
                .target
                .is_none_or(|value| row.target_id == Some(value))
        })
        .filter(|row| {
            query.categories.is_empty()
                || query
                    .categories
                    .iter()
                    .any(|value| value == category_for(row.action_type))
        })
        .filter(|row| {
            query.search.as_ref().is_none_or(|needle| {
                row.reason
                    .as_deref()
                    .is_some_and(|reason| reason.to_lowercase().contains(needle))
            })
        })
        .filter(|row| !query.hide_noise || !is_noise(row.user_id, row.action_type, bot_user_id))
        .collect::<Vec<_>>();

    let mut counts = BTreeMap::from([
        ("moderation", 0),
        ("rollen", 0),
        ("sonstiges", 0),
        ("struktur", 0),
    ]);
    for row in &filtered {
        *counts.entry(category_for(row.action_type)).or_default() += 1;
    }
    let total = filtered.len();
    let page = filtered
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect::<Vec<_>>();
    let user_ids = page
        .iter()
        .flat_map(|row| [row.user_id, row.target_id])
        .flatten()
        .filter_map(|id| u64::try_from(id).ok())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let names = app.names().resolve(&user_ids).await;
    let named = |id: Option<i64>| NamedId {
        id: id.map(Snowflake),
        name: id
            .and_then(|value| u64::try_from(value).ok())
            .and_then(|value| names.get(&value).cloned()),
    };
    let entries = page
        .into_iter()
        .map(|row| AuditEntry {
            entry_id: Snowflake(row.entry_id),
            occurred_at: row.occurred_at.to_rfc3339(),
            action_type: row.action_type,
            action_name: action_name(row.action_type),
            category: category_for(row.action_type),
            actor: named(row.user_id),
            target: named(row.target_id),
            changes: row.changes.unwrap_or(Value::Null),
            options: row.options.unwrap_or(Value::Null),
            reason: row.reason,
            is_noise: is_noise(row.user_id, row.action_type, bot_user_id),
        })
        .collect();

    match serde_json::to_value(AuditResponse {
        entries,
        total,
        counts,
        noise_hidden: if query.hide_noise { noise_count } else { 0 },
    }) {
        Ok(value) => ok_json(value),
        Err(err) => {
            tracing::error!(%err, "Audit-Log-Antwort konnte nicht serialisiert werden");
            err_text(500, "Audit log unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::collections::HashMap;

    const BOT_ID: i64 = 1_355_078_189_894_078_597;

    #[test]
    fn bot_role_update_is_noise() {
        assert!(is_noise(Some(BOT_ID), 25, BOT_ID));
    }

    #[test]
    fn bot_ban_is_not_noise() {
        assert!(!is_noise(Some(BOT_ID), 22, BOT_ID));
    }

    #[test]
    fn human_role_update_is_not_noise() {
        assert!(!is_noise(Some(42), 25, BOT_ID));
    }

    #[test]
    fn bot_member_update_is_not_noise() {
        assert!(!is_noise(Some(BOT_ID), 24, BOT_ID));
    }

    #[test]
    fn maps_categories_with_unknown_fallback() {
        assert_eq!(category_for(22), "moderation");
        assert_eq!(category_for(25), "rollen");
        assert_eq!(category_for(110), "struktur");
        assert_eq!(category_for(999), "sonstiges");
    }

    #[test]
    fn serializes_snowflake_as_json_string() {
        let value = serde_json::to_value(Snowflake(BOT_ID)).expect("Snowflake serialisieren");
        assert_eq!(value, serde_json::Value::String(BOT_ID.to_string()));
    }

    #[test]
    fn parses_filters_and_caps_limit() {
        let params = HashMap::from([
            ("from".into(), "2026-06-01".into()),
            ("to".into(), "2026-06-30".into()),
            ("category".into(), "moderation,rollen".into()),
            ("action_type".into(), "22".into()),
            ("actor".into(), "42".into()),
            ("target".into(), "43".into()),
            ("q".into(), "grund".into()),
            ("noise".into(), "show".into()),
            ("limit".into(), "999".into()),
            ("offset".into(), "7".into()),
        ]);

        let query = parse_params(
            params,
            NaiveDate::from_ymd_opt(2026, 7, 14).expect("gültiges Datum"),
        )
        .expect("gültige Filter");

        assert_eq!(query.from.to_string(), "2026-06-01");
        assert_eq!(query.to.to_string(), "2026-07-01");
        assert_eq!(query.categories, vec!["moderation", "rollen"]);
        assert_eq!(query.action_type, Some(22));
        assert_eq!(query.actor, Some(42));
        assert_eq!(query.target, Some(43));
        assert_eq!(query.search.as_deref(), Some("grund"));
        assert!(!query.hide_noise);
        assert_eq!(query.limit, 500);
        assert_eq!(query.offset, 7);
    }

    #[test]
    fn rejects_overflowing_to_date() {
        let params = HashMap::from([("to".into(), "+262142-12-31".into())]);
        assert!(parse_params(
            params,
            NaiveDate::from_ymd_opt(2026, 7, 14).expect("gültiges Datum")
        )
        .is_err());
    }
}
