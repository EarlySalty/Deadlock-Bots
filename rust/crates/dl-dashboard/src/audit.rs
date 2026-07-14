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
const ALWAYS_NOISE_ACTION_TYPES: &[i32] = &[40, 41, 42, 192, 193];

const MODERATION_TYPES: &[i32] = &[
    20, 21, 22, 23, 24, 26, 27, 72, 73, 74, 75, 140, 141, 142, 143, 144, 145,
];
const ROLE_TYPES: &[i32] = &[25, 30, 31, 32];
const STRUCTURE_TYPES: &[i32] = &[
    1, 10, 11, 12, 13, 14, 15, 40, 41, 42, 50, 51, 52, 60, 61, 62, 80, 81, 82, 83, 84, 85, 90, 91,
    92, 100, 101, 102, 110, 111, 112, 121, 130, 131, 132, 163, 164, 165, 166, 167, 190, 191, 192,
    193,
];

pub const CATEGORY_ACTION_TYPES: &[(&str, &[i32])] = &[
    ("moderation", MODERATION_TYPES),
    ("rollen", ROLE_TYPES),
    ("struktur", STRUCTURE_TYPES),
];

pub fn is_noise(
    actor_id: Option<i64>,
    target_id: Option<i64>,
    action_type: i32,
    bot_user_id: i64,
) -> bool {
    (actor_id == Some(bot_user_id) && NOISE_ACTION_TYPES.contains(&action_type))
        || (action_type == 25 && actor_id.is_some() && actor_id == target_id)
        || ALWAYS_NOISE_ACTION_TYPES.contains(&action_type)
}

pub fn category_for(action_type: i32) -> &'static str {
    CATEGORY_ACTION_TYPES
        .iter()
        .find_map(|(category, types)| types.contains(&action_type).then_some(*category))
        .unwrap_or("sonstiges")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    User,
    Channel,
    Role,
    Message,
    Webhook,
    Emoji,
    Integration,
    Sonstiges,
}

pub fn target_kind(action_type: i32) -> TargetKind {
    match action_type {
        20 | 22 | 23 | 24 | 25 | 26 | 27 | 72 | 73 | 74 | 75 | 143 | 144 | 145 => TargetKind::User,
        10 | 11 | 12 | 13 | 14 | 15 | 110 | 111 | 112 => TargetKind::Channel,
        30..=32 => TargetKind::Role,
        50..=52 => TargetKind::Webhook,
        60..=62 => TargetKind::Emoji,
        80..=82 => TargetKind::Integration,
        _ => TargetKind::Sonstiges,
    }
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

#[derive(Default)]
struct TargetLexicons {
    roles: HashMap<i64, String>,
    channels: HashMap<i64, String>,
}

fn value_id(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|value| value.parse().ok())
        .or_else(|| value.as_i64())
}

fn changed_name(changes: Option<&Value>) -> Option<&str> {
    changes?
        .as_array()?
        .iter()
        .find(|change| change.get("key").and_then(Value::as_str) == Some("name"))
        .and_then(|change| {
            change
                .get("new_value")
                .and_then(Value::as_str)
                .or_else(|| change.get("old_value").and_then(Value::as_str))
        })
        .filter(|name| !name.trim().is_empty())
}

fn build_target_lexicons(rows: &[AuditRow]) -> TargetLexicons {
    let mut lexicons = TargetLexicons::default();

    for row in rows {
        if row.action_type == 25 {
            for role in row
                .changes
                .as_ref()
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|change| change.get("new_value").and_then(Value::as_array))
                .flatten()
            {
                if let (Some(id), Some(name)) = (
                    role.get("id").and_then(value_id),
                    role.get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty()),
                ) {
                    lexicons.roles.entry(id).or_insert_with(|| name.to_string());
                }
            }
        }

        let Some(name) = changed_name(row.changes.as_ref()) else {
            continue;
        };
        match target_kind(row.action_type) {
            TargetKind::Role => {
                if let Some(id) = row.target_id {
                    lexicons.roles.entry(id).or_insert_with(|| name.to_string());
                }
            }
            TargetKind::Channel => {
                if let Some(id) = row.target_id {
                    lexicons
                        .channels
                        .entry(id)
                        .or_insert_with(|| name.to_string());
                }
            }
            _ => {}
        }
        if let Some(id) = row
            .options
            .as_ref()
            .and_then(|options| options.get("channel_id"))
            .and_then(value_id)
        {
            lexicons
                .channels
                .entry(id)
                .or_insert_with(|| name.to_string());
        }
    }

    lexicons
}

#[derive(Serialize)]
struct NamedId {
    id: Option<Snowflake>,
    name: Option<String>,
}

#[derive(Serialize)]
struct TargetId {
    id: Option<Snowflake>,
    name: Option<String>,
    kind: TargetKind,
}

#[derive(Serialize)]
struct AuditEntry {
    entry_id: Snowflake,
    occurred_at: String,
    action_type: i32,
    action_name: String,
    category: &'static str,
    actor: NamedId,
    target: TargetId,
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
        21 => "Mitglieder aufgeräumt (Prune)",
        22 => "Mitglied gebannt",
        23 => "Bann aufgehoben",
        24 => "Mitglied geändert (Mute/Timeout/Name)",
        25 => "Rollen geändert",
        26 => "Mitglied verschoben",
        27 => "Mitglied getrennt",
        28 => "Bot hinzugefügt",
        30 => "Rolle erstellt",
        31 => "Rolle geändert",
        32 => "Rolle gelöscht",
        40 => "Einladung erstellt",
        41 => "Einladung geändert",
        42 => "Einladung gelöscht",
        50 => "Webhook erstellt",
        51 => "Webhook geändert",
        52 => "Webhook gelöscht",
        60 => "Emoji erstellt",
        61 => "Emoji geändert",
        62 => "Emoji gelöscht",
        72 => "Nachricht gelöscht",
        73 => "Nachrichten gesammelt gelöscht",
        74 => "Nachricht angepinnt",
        75 => "Pin entfernt",
        80 => "Integration erstellt",
        81 => "Integration geändert",
        82 => "Integration gelöscht",
        83 => "Bühne erstellt",
        84 => "Bühne geändert",
        85 => "Bühne gelöscht",
        90 => "Sticker erstellt",
        91 => "Sticker geändert",
        92 => "Sticker gelöscht",
        100 => "Event erstellt",
        101 => "Event geändert",
        102 => "Event gelöscht",
        110 => "Thread erstellt",
        111 => "Thread geändert",
        112 => "Thread gelöscht",
        121 => "Befehlsrechte geändert",
        130 => "Soundboard-Sound erstellt",
        131 => "Soundboard-Sound geändert",
        132 => "Soundboard-Sound gelöscht",
        140 => "AutoMod-Regel erstellt",
        141 => "AutoMod-Regel geändert",
        142 => "AutoMod-Regel gelöscht",
        143 => "AutoMod hat Nachricht blockiert",
        144 => "AutoMod hat gemeldet",
        145 => "AutoMod hat stummgeschaltet",
        163 => "Onboarding-Frage erstellt",
        164 => "Onboarding-Frage geändert",
        165 => "Onboarding-Frage gelöscht",
        166 => "Onboarding erstellt",
        167 => "Onboarding geändert",
        190 => "Startseite erstellt",
        191 => "Startseite geändert",
        192 => "Kanal-Status gesetzt",
        193 => "Kanal-Status entfernt",
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

    // ponytail: Für die Namenslexika werden alle ~26k Zeilen gebraucht; bei >1 Mio. materialisieren.
    let rows = match sqlx::query_as::<_, AuditRow>(
        r#"
        SELECT entry_id, action_type, user_id, target_id, changes, options, reason, occurred_at
          FROM core.discord_audit_log
         ORDER BY occurred_at DESC, entry_id DESC
        "#,
    )
    .fetch_all(app.pool())
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(%err, "Audit-Log konnte nicht geladen werden");
            return err_text(500, "Audit log unavailable");
        }
    };

    let lexicons = build_target_lexicons(&rows);
    let dated = rows
        .into_iter()
        .filter(|row| {
            let date = row.occurred_at.date_naive();
            date >= query.from && date < query.to
        })
        .collect::<Vec<_>>();
    let noise_count = dated
        .iter()
        .filter(|row| is_noise(row.user_id, row.target_id, row.action_type, bot_user_id))
        .count();
    let filtered = dated
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
        .filter(|row| {
            !query.hide_noise || !is_noise(row.user_id, row.target_id, row.action_type, bot_user_id)
        })
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
        .flat_map(|row| {
            [
                row.user_id,
                (target_kind(row.action_type) == TargetKind::User)
                    .then_some(row.target_id)
                    .flatten(),
            ]
        })
        .flatten()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let names = if user_ids.is_empty() {
        HashMap::new()
    } else {
        match sqlx::query_as::<_, (i64, Option<String>)>(
            r#"
            SELECT discord_id,
                   COALESCE(NULLIF(BTRIM(global_name), ''), NULLIF(BTRIM(username), ''))
              FROM core.users
             WHERE discord_id = ANY($1)
            "#,
        )
        .bind(&user_ids)
        .fetch_all(app.pool())
        .await
        {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(id, name)| name.map(|name| (id, name)))
                .collect(),
            Err(err) => {
                tracing::warn!(%err, "Audit-Log-Namen konnten nicht geladen werden");
                HashMap::new()
            }
        }
    };
    let named = |id: Option<i64>| NamedId {
        id: id.map(Snowflake),
        name: id.and_then(|value| names.get(&value).cloned()),
    };
    let entries = page
        .into_iter()
        .map(|row| {
            let kind = target_kind(row.action_type);
            let target_name = row.target_id.and_then(|id| match kind {
                TargetKind::User => names.get(&id),
                TargetKind::Role => lexicons.roles.get(&id),
                TargetKind::Channel => lexicons.channels.get(&id),
                _ => None,
            });
            AuditEntry {
                entry_id: Snowflake(row.entry_id),
                occurred_at: row.occurred_at.to_rfc3339(),
                action_type: row.action_type,
                action_name: action_name(row.action_type),
                category: category_for(row.action_type),
                actor: named(row.user_id),
                target: TargetId {
                    id: row.target_id.map(Snowflake),
                    name: target_name.cloned(),
                    kind,
                },
                changes: row.changes.unwrap_or(Value::Null),
                options: row.options.unwrap_or(Value::Null),
                reason: row.reason,
                is_noise: is_noise(row.user_id, row.target_id, row.action_type, bot_user_id),
            }
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
        assert!(is_noise(Some(BOT_ID), Some(42), 25, BOT_ID));
    }

    #[test]
    fn self_assigned_role_is_noise() {
        assert!(is_noise(Some(42), Some(42), 25, BOT_ID));
    }

    #[test]
    fn bot_ban_is_not_noise() {
        assert!(!is_noise(Some(BOT_ID), Some(42), 22, BOT_ID));
    }

    #[test]
    fn role_assigned_to_another_user_is_not_noise() {
        assert!(!is_noise(Some(42), Some(43), 25, BOT_ID));
    }

    #[test]
    fn bot_member_update_is_not_noise() {
        assert!(!is_noise(Some(BOT_ID), Some(42), 24, BOT_ID));
    }

    #[test]
    fn invitations_and_voice_channel_status_are_always_noise() {
        for action_type in [40, 192, 193] {
            assert!(is_noise(Some(42), Some(43), action_type, BOT_ID));
        }
    }

    #[test]
    fn maps_action_names_with_unknown_fallback() {
        assert_eq!(action_name(40), "Einladung erstellt");
        assert_eq!(action_name(50), "Webhook erstellt");
        assert_eq!(action_name(192), "Kanal-Status gesetzt");
        assert_eq!(action_name(999), "Unbekannt (999)");
    }

    #[test]
    fn maps_categories_with_unknown_fallback() {
        assert_eq!(category_for(40), "struktur");
        assert_eq!(category_for(22), "moderation");
        assert_eq!(category_for(25), "rollen");
        assert_eq!(category_for(110), "struktur");
        assert_eq!(category_for(999), "sonstiges");
    }

    #[test]
    fn maps_target_kind_for_every_supported_group() {
        for action_type in [20, 22, 23, 24, 25, 26, 27, 72, 73, 74, 75, 143, 144, 145] {
            assert_eq!(target_kind(action_type), TargetKind::User);
        }
        for action_type in [10, 11, 12, 13, 14, 15, 110, 111, 112] {
            assert_eq!(target_kind(action_type), TargetKind::Channel);
        }
        for action_type in [30, 31, 32] {
            assert_eq!(target_kind(action_type), TargetKind::Role);
        }
        for action_type in [50, 51, 52] {
            assert_eq!(target_kind(action_type), TargetKind::Webhook);
        }
        for action_type in [60, 61, 62] {
            assert_eq!(target_kind(action_type), TargetKind::Emoji);
        }
        for action_type in [80, 81, 82] {
            assert_eq!(target_kind(action_type), TargetKind::Integration);
        }
        for action_type in [40, 41, 42] {
            assert_eq!(target_kind(action_type), TargetKind::Sonstiges);
        }
        assert_eq!(target_kind(999), TargetKind::Sonstiges);
    }

    fn lexicon_row(
        entry_id: i64,
        action_type: i32,
        target_id: Option<i64>,
        changes: Option<Value>,
    ) -> AuditRow {
        AuditRow {
            entry_id,
            action_type,
            user_id: Some(42),
            target_id,
            changes,
            options: None,
            reason: None,
            occurred_at: DateTime::from_timestamp(entry_id, 0).expect("gültige Testzeit"),
        }
    }

    #[test]
    fn lexicons_resolve_role_and_channel_rows_without_own_name() {
        let rows = vec![
            lexicon_row(400, 31, Some(700), None),
            lexicon_row(
                300,
                30,
                Some(700),
                Some(serde_json::json!([
                    {"key": "name", "new_value": "Moderation"}
                ])),
            ),
            lexicon_row(200, 11, Some(800), None),
            lexicon_row(
                100,
                10,
                Some(800),
                Some(serde_json::json!([
                    {"key": "name", "new_value": "einsatzleitung"}
                ])),
            ),
            lexicon_row(
                50,
                25,
                Some(42),
                Some(serde_json::json!([
                    {"key": "$add", "new_value": [{"id": "900", "name": "Einsatzteam"}]}
                ])),
            ),
        ];

        let lexicons = build_target_lexicons(&rows);

        assert_eq!(
            lexicons.roles.get(&700).map(String::as_str),
            Some("Moderation")
        );
        assert_eq!(
            lexicons.roles.get(&900).map(String::as_str),
            Some("Einsatzteam")
        );
        assert_eq!(
            lexicons.channels.get(&800).map(String::as_str),
            Some("einsatzleitung")
        );
    }

    #[test]
    fn serializes_snowflake_as_json_string() {
        let value = serde_json::to_value(Snowflake(BOT_ID)).expect("Snowflake serialisieren");
        assert_eq!(value, serde_json::Value::String(BOT_ID.to_string()));
    }

    #[test]
    fn named_actor_and_target_keep_their_string_ids() {
        let actor = serde_json::to_value(NamedId {
            id: Some(Snowflake(42)),
            name: Some("Admin".to_string()),
        })
        .expect("Actor serialisieren");
        let target = serde_json::to_value(TargetId {
            id: Some(Snowflake(700)),
            name: Some("Moderation".to_string()),
            kind: TargetKind::Role,
        })
        .expect("Ziel serialisieren");

        assert_eq!(actor["id"], "42");
        assert_eq!(actor["name"], "Admin");
        assert_eq!(target["id"], "700");
        assert_eq!(target["name"], "Moderation");
        assert_eq!(target["kind"], "role");
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
