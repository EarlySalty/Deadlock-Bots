//! Versionierter Admin-Vertrag. Nur Einträge im geprüften Katalog sind schreibbar.
//! Ganzzahlen laufen als Dezimalstrings über JSON, niemals als JavaScript-Number.
use crate::bot_config as bc;
use crate::runtime_config as rc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Boolean,
    Integer,
    Number,
    String,
    IntegerList,
    StringList,
    Choice,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub path: String,
    pub group: String,
    pub label: String,
    pub kind: Kind,
    pub nullable: bool,
    pub writable: bool,
    pub restart_required: bool,
    pub help: String,
    pub min: Option<String>,
    pub max: Option<String>,
    pub length: Option<usize>,
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub fields: Vec<Field>,
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequest {
    pub revision: String,
    pub changes: BTreeMap<String, Value>,
}

#[derive(Debug)]
pub struct CatalogError(pub &'static str);
impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for CatalogError {}
fn invalid() -> CatalogError {
    CatalogError("Ungültige Einstellung: Typ, Wertebereich und Pflichtfelder prüfen.")
}

type Bounds = (i64, i64);

const fn u64_bounds(range: &std::ops::RangeInclusive<u64>) -> Bounds {
    assert!(*range.end() <= i64::MAX as u64);
    (*range.start() as i64, *range.end() as i64)
}

const fn usize_bounds(range: &std::ops::RangeInclusive<usize>) -> Bounds {
    assert!(*range.end() <= i64::MAX as usize);
    (*range.start() as i64, *range.end() as i64)
}

const fn u32_bounds(range: &std::ops::RangeInclusive<u32>) -> Bounds {
    (*range.start() as i64, *range.end() as i64)
}

const fn u16_bounds(range: &std::ops::RangeInclusive<u16>) -> Bounds {
    (*range.start() as i64, *range.end() as i64)
}

const fn i64_bounds(range: &std::ops::RangeInclusive<i64>) -> Bounds {
    (*range.start(), *range.end())
}

const DISCORD_ID_PATHS: &[&str] = &[
    "twitch_invites.excluded_voice_channel_ids",
    "runtime.start.command_guild_id",
    "runtime.start.mcp_guild_id",
    "runtime.bridges.matcher_guild_id",
    "runtime.bridges.matcher_notify_channel_id",
    "runtime.bridges.matcher_role_id",
    "runtime.dashboard.audit_bot_user_id",
    "runtime.dashboard.insights_guild_id",
    "runtime.community.lfg_panel_channel_id",
    "runtime.community.lfg_forum_channel_id",
    "runtime.community.lfg_freetext_channel_id",
    "runtime.community.scrim_category_id",
    "runtime.community.concierge_fallback_category_id",
    "runtime.community.concierge_pate_category_id",
    "runtime.community.concierge_pate_channel_id",
    "runtime.community.concierge_pate_panel_channel_id",
    "runtime.community.concierge_pate_help_channel_id",
    "runtime.community.concierge_mod_role_id",
    "runtime.community.concierge_test_users",
    "runtime.moderation.channel_id",
    "runtime.moderation.scan_channel_ids",
    "runtime.ai.transparency_channel_id",
    "runtime.ai.brain_channels",
];

const LOADER_BOUNDS: &[(&[&str], Bounds)] = &[
    (DISCORD_ID_PATHS, u64_bounds(&rc::DISCORD_ID_RANGE)),
    (
        &[
            "runtime.community.router_auto_move_delay_seconds",
            "runtime.ai.transparency_error_repeat_seconds",
            "runtime.ai.transparency_error_followup_seconds",
            "runtime.ai.brain_cooldown_seconds",
        ],
        u64_bounds(&rc::RUNTIME_DELAY_SECONDS_RANGE),
    ),
    (
        &[
            "runtime.dashboard.session_ttl_seconds",
            "runtime.dashboard.oauth_state_ttl_seconds",
            "runtime.community.survey_interval_days",
            "runtime.community.concierge_active_minutes",
            "runtime.moderation.timeout_minutes",
            "runtime.moderation.behavior_proposal_timeout_minutes",
        ],
        i64_bounds(&rc::RUNTIME_SPAN_RANGE),
    ),
    (
        &["runtime.start.broker_idempotency_max_entries"],
        usize_bounds(&rc::BROKER_IDEMPOTENCY_MAX_ENTRIES_RANGE),
    ),
    (
        &["runtime.ai.brain_daily_user_limit"],
        usize_bounds(&rc::BRAIN_DAILY_USER_LIMIT_RANGE),
    ),
    (
        &["runtime.ai.brain_max_question_len"],
        usize_bounds(&rc::BRAIN_MAX_QUESTION_LEN_RANGE),
    ),
    (
        &["runtime.ai.brain_image_max_count"],
        usize_bounds(&rc::BRAIN_IMAGE_MAX_COUNT_RANGE),
    ),
    (
        &["runtime.ai.brain_image_jpeg_quality"],
        u64_bounds(&rc::BRAIN_IMAGE_JPEG_QUALITY_RANGE),
    ),
    (
        &["runtime.ai.brain_image_max_download_bytes"],
        u64_bounds(&rc::BRAIN_IMAGE_MAX_DOWNLOAD_BYTES_RANGE),
    ),
    (
        &["runtime.ai.brain_image_max_edge"],
        u64_bounds(&rc::BRAIN_IMAGE_MAX_EDGE_RANGE),
    ),
    (
        &[
            "runtime.bridges.matcher_auto_threshold",
            "runtime.bridges.matcher_review_threshold",
        ],
        i64_bounds(&rc::MATCHER_THRESHOLD_RANGE),
    ),
    (
        &["runtime.bridges.matcher_max_ai_per_scan"],
        u64_bounds(&rc::MATCHER_MAX_AI_PER_SCAN_RANGE),
    ),
    (
        &["runtime.bridges.matcher_scan_interval_hours"],
        u64_bounds(&rc::MATCHER_SCAN_INTERVAL_HOURS_RANGE),
    ),
    (
        &["runtime.community.supporter_role_sync_max_revokes"],
        u64_bounds(&rc::SUPPORTER_MAX_REVOKES_RANGE),
    ),
    (
        &["runtime.community.supporter_sync_interval_seconds"],
        u64_bounds(&rc::SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE),
    ),
    (
        &[SUPPORTER_TIERS_PATH],
        u64_bounds(&(rc::SUPPORTER_TIER_MIN_EUR..=rc::SUPPORTER_TIER_MAX_EUR)),
    ),
    (
        &["runtime.community.supporter_funding_goal_cents"],
        i64_bounds(&rc::SUPPORTER_FUNDING_GOAL_CENTS_RANGE),
    ),
    (
        &[
            "twitch_invites.personal_links_per_channel",
            "twitch_invites.guild_invite_reserve",
        ],
        u16_bounds(&bc::TWITCH_INVITE_LINKS_RANGE),
    ),
    (
        &["twitch_invites.evaluation_interval_seconds"],
        u64_bounds(&bc::TWITCH_INVITE_EVALUATION_INTERVAL_SECONDS_RANGE),
    ),
    (
        &["concierge.timeout_seconds"],
        u64_bounds(&bc::CONCIERGE_TIMEOUT_SECONDS_RANGE),
    ),
    (
        &["tempvoice.empty_lane_grace_seconds"],
        u64_bounds(&bc::TEMPVOICE_EMPTY_LANE_GRACE_SECONDS_RANGE),
    ),
];

const LOADER_NUMBER_BOUNDS: &[(&[&str], &std::ops::RangeInclusive<f64>)] = &[
    (
        &[
            "runtime.start.broker_idempotency_ttl_seconds",
            "runtime.start.broker_inflight_ttl_seconds",
            "runtime.start.broker_waiter_timeout_seconds",
        ],
        &rc::BROKER_SECONDS_RANGE,
    ),
    (
        &["runtime.bridges.twitch_timeout_seconds"],
        &rc::TWITCH_TIMEOUT_SECONDS_RANGE,
    ),
    (
        &[
            "runtime.bridges.matcher_fuzzy_floor",
            "runtime.moderation.analyze_flag_threshold",
            "runtime.moderation.auto_verify_threshold",
            "runtime.moderation.propose_verify_threshold",
        ],
        &rc::UNIT_INTERVAL_RANGE,
    ),
];

const LOADER_TEXT_MAX_CHARS: &[(&[&str], usize)] = &[(
    &["runtime.start.presence_status"],
    rc::PRESENCE_STATUS_MAX_CHARS,
)];

type TextRule = fn(&str) -> bool;

const LOADER_TEXT_RULES: &[(&[&str], TextRule)] = &[
    (&["discord.guild_id"], bc::snowflake),
    (&["runtime.start.log_filter"], rc::log_filter_valid),
    (&["runtime.start.command_prefix"], rc::command_prefix_valid),
    (
        &["runtime.start.presence_status"],
        rc::presence_status_valid,
    ),
];

const LLM_USE_CASE_PREFIX: &str = "llm.use_cases.";

const LLM_USE_CASE_BOUNDS: &[(&str, Bounds)] = &[
    (
        ".max_output_tokens",
        u32_bounds(&bc::LLM_MAX_OUTPUT_TOKENS_RANGE),
    ),
    (
        ".request_timeout_seconds",
        u64_bounds(&bc::LLM_REQUEST_TIMEOUT_SECONDS_RANGE),
    ),
];

const LLM_USE_CASE_NUMBER_BOUNDS: &[(&str, &std::ops::RangeInclusive<f64>)] =
    &[(".temperature", &bc::LLM_TEMPERATURE_RANGE)];

fn path_bound<T: Copy>(table: &[(&[&str], T)], path: &str) -> Option<T> {
    table
        .iter()
        .find(|(paths, _)| paths.contains(&path))
        .map(|(_, bound)| *bound)
}

fn llm_bound<T: Copy>(table: &[(&str, T)], path: &str) -> Option<T> {
    let rest = path.strip_prefix(LLM_USE_CASE_PREFIX)?;
    table
        .iter()
        .find(|(suffix, _)| rest.ends_with(suffix))
        .map(|(_, bound)| *bound)
}

fn integer_bounds(path: &str) -> Option<Bounds> {
    path_bound(LOADER_BOUNDS, path).or_else(|| llm_bound(LLM_USE_CASE_BOUNDS, path))
}

fn number_bounds(path: &str) -> Option<&'static std::ops::RangeInclusive<f64>> {
    path_bound(LOADER_NUMBER_BOUNDS, path).or_else(|| llm_bound(LLM_USE_CASE_NUMBER_BOUNDS, path))
}

fn text_values(path: &str) -> Vec<(&'static str, String)> {
    match path {
        "runtime.community.supporter_role_sync_max_revokes" => {
            vec![("{default}", rc::SUPPORTER_DEFAULT_MAX_REVOKES.to_string())]
        }
        "runtime.community.supporter_sync_interval_seconds" => vec![(
            "{default}",
            rc::SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS.to_string(),
        )],
        "runtime.community.supporter_funding_goal_cents" => vec![
            (
                "{default}",
                rc::SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS.to_string(),
            ),
            (
                "{default_eur}",
                (rc::SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS / 100).to_string(),
            ),
        ],
        SUPPORTER_TIERS_PATH => vec![
            (
                "{default}",
                rc::SUPPORTER_DEFAULT_TIERS_EUR
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ("{max_count}", rc::SUPPORTER_MAX_TIERS.to_string()),
        ],
        _ => vec![],
    }
}

fn german_number(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push('.');
        }
        out.push(digit);
    }
    out
}

fn german_decimal(value: f64) -> String {
    value.to_string().replace('.', ",")
}

struct FieldBounds {
    min: Option<String>,
    max: Option<String>,
    texts: Vec<(&'static str, String)>,
}

fn german_bound(name: &'static str, bound: i64) -> Option<(&'static str, String)> {
    Some((name, german_number(u64::try_from(bound).ok()?)))
}

fn field_bounds(path: &str, type_bounds: Option<Bounds>) -> FieldBounds {
    if let Some(range) = number_bounds(path) {
        return FieldBounds {
            min: Some(range.start().to_string()),
            max: Some(range.end().to_string()),
            texts: vec![
                ("{min}", german_decimal(*range.start())),
                ("{max}", german_decimal(*range.end())),
            ],
        };
    }
    if let Some(max_chars) = path_bound(LOADER_TEXT_MAX_CHARS, path) {
        return FieldBounds {
            min: None,
            max: Some(max_chars.to_string()),
            texts: vec![("{max}", german_number(max_chars as u64))],
        };
    }
    let Some((min, max)) = integer_bounds(path).or(type_bounds) else {
        return FieldBounds {
            min: None,
            max: None,
            texts: vec![],
        };
    };
    FieldBounds {
        min: Some(min.to_string()),
        max: Some(max.to_string()),
        texts: [german_bound("{min}", min), german_bound("{max}", max)]
            .into_iter()
            .flatten()
            .collect(),
    }
}

/// TSV ist eine explizite, versionierte Erlaubnisliste, keine Reflexion über
/// beliebige Nutzereingaben. Neue Config-Felder bleiben bis zur Prüfung gesperrt.
pub fn fields(spec: &str, accounts: &[i16]) -> Vec<Field> {
    let mut result = Vec::new();
    for line in spec
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 6, "Katalogeintrag hat sechs Spalten");
        let keys: Vec<Option<i16>> = if columns[0].contains("{account}") {
            accounts.iter().copied().map(Some).collect()
        } else {
            vec![None]
        };
        for account in keys {
            let replace = |text: &str| match account {
                Some(id) => text.replace("{account}", &id.to_string()),
                None => text.to_owned(),
            };
            let mut ty = columns[1];
            let nullable = ty.starts_with("Option<");
            if nullable {
                ty = &ty[7..ty.len() - 1];
            }
            let (kind, integer_type, length, choices) = if ty.starts_with("Vec<") {
                let inner = &ty[4..ty.len() - 1];
                if inner == "String" {
                    (Kind::StringList, "", None, vec![])
                } else {
                    (Kind::IntegerList, inner, None, vec![])
                }
            } else if ty.starts_with('[') {
                let (inner, n) = ty[1..ty.len() - 1].split_once(';').expect("festes Array");
                (
                    Kind::IntegerList,
                    inner.trim(),
                    Some(n.trim().parse().expect("Arraylänge")),
                    vec![],
                )
            } else {
                match ty {
                    "bool" => (Kind::Boolean, "", None, vec![]),
                    "f64" => (Kind::Number, "", None, vec![]),
                    "u8" | "u16" | "u32" | "u64" | "usize" | "i16" | "i64" => {
                        (Kind::Integer, ty, None, vec![])
                    }
                    "Provider" => (
                        Kind::Choice,
                        "",
                        None,
                        vec!["fireworks".into(), "openai".into()],
                    ),
                    "ReasoningEffort" => (
                        Kind::Choice,
                        "",
                        None,
                        vec!["none".into(), "low".into(), "medium".into(), "high".into()],
                    ),
                    "DiscordMode" => (Kind::Choice, "", None, vec!["broker".into(), "noop".into()]),
                    "CommandScope" => (
                        Kind::Choice,
                        "",
                        None,
                        vec!["guild".into(), "global".into(), "both".into()],
                    ),
                    _ => (Kind::String, "", None, vec![]),
                }
            };
            let type_bounds = match integer_type {
                "u8" => Some((0i64, 255i64)),
                "u16" => Some((0, 65535)),
                "u32" => Some((0, u32::MAX as i64)),
                "u64" | "usize" => Some((0, i64::MAX)),
                "i16" => Some((i16::MIN as i64, i16::MAX as i64)),
                "i64" => Some((i64::MIN, i64::MAX)),
                "" => None,
                _ => panic!("nicht klassifizierter Ganzzahltyp"),
            };
            let path = replace(columns[0]);
            let writable = columns[4] == "edit";
            let bounds = if writable {
                field_bounds(&path, type_bounds)
            } else {
                FieldBounds {
                    min: None,
                    max: None,
                    texts: vec![],
                }
            };
            let values: Vec<_> = bounds.texts.into_iter().chain(text_values(&path)).collect();
            let fill = |text: String| {
                values
                    .iter()
                    .fold(text, |text, (name, value)| text.replace(name, value))
            };
            result.push(Field {
                path,
                group: replace(columns[2]),
                label: fill(replace(columns[3])),
                kind,
                nullable,
                writable,
                restart_required: true,
                help: fill(replace(columns[5])),
                min: bounds.min,
                max: bounds.max,
                length,
                choices,
            });
        }
    }
    let unique: BTreeSet<_> = result.iter().map(|field| &field.path).collect();
    assert_eq!(unique.len(), result.len(), "eindeutige Katalogpfade");
    result
}

fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = if let Some(array) = current.as_array() {
            let id = part.parse::<i64>().ok()?;
            array
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_i64) == Some(id))?
        } else {
            current.get(part)?
        };
    }
    Some(current)
}
fn integer_to_wire(value: &Value) -> Value {
    if value.is_null() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}
pub fn build(value: &Value, fields: Vec<Field>) -> Catalog {
    let mut values = BTreeMap::new();
    for field in &fields {
        // Geschützte Infrastruktur-/Secret-Referenzwerte werden nicht an den Browser geliefert.
        if !field.writable {
            continue;
        }
        let value = value_at(value, &field.path).unwrap_or(&Value::Null);
        let wire = match field.kind {
            Kind::Integer => integer_to_wire(value),
            Kind::IntegerList if value.is_array() => Value::Array(
                value
                    .as_array()
                    .expect("Array")
                    .iter()
                    .map(integer_to_wire)
                    .collect(),
            ),
            _ => value.clone(),
        };
        values.insert(field.path.clone(), wire);
    }
    Catalog {
        version: 1,
        fields,
        values,
    }
}

fn integer(field: &Field, value: &Value) -> Result<Value, CatalogError> {
    let raw = value.as_str().ok_or_else(invalid)?;
    let parsed = raw.parse::<i64>().map_err(|_| invalid())?;
    if parsed.to_string() != raw {
        return Err(invalid());
    }
    let min = field
        .min
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    let max = field
        .max
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    if !(min..=max).contains(&parsed) {
        return Err(invalid());
    }
    Ok(parsed.into())
}
fn text(field: &Field, value: &Value) -> Result<Value, CatalogError> {
    let raw = value.as_str().ok_or_else(invalid)?;
    let max_chars = field
        .max
        .as_deref()
        .map(str::parse::<usize>)
        .transpose()
        .map_err(|_| invalid())?;
    if raw.len() > 2048
        || raw.chars().any(char::is_control)
        || max_chars.is_some_and(|max| raw.chars().count() > max)
        || path_bound(LOADER_TEXT_RULES, &field.path).is_some_and(|valid| !valid(raw))
    {
        return Err(invalid());
    }
    Ok(value.clone())
}
fn number(field: &Field, value: &Value) -> Result<Value, CatalogError> {
    const MAX_EXACT_JSON_INTEGER: u64 = (1u64 << 53) - 1;
    let raw = value.as_number().ok_or_else(invalid)?;
    let parsed = if let Some(integer) = raw.as_i64() {
        if integer.unsigned_abs() > MAX_EXACT_JSON_INTEGER {
            return Err(invalid());
        }
        integer as f64
    } else if let Some(integer) = raw.as_u64() {
        if integer > MAX_EXACT_JSON_INTEGER {
            return Err(invalid());
        }
        integer as f64
    } else {
        raw.as_f64().ok_or_else(invalid)?
    };
    if !parsed.is_finite()
        || !field
            .min
            .as_deref()
            .and_then(|v| v.parse::<f64>().ok())
            .is_none_or(|min| parsed >= min)
        || !field
            .max
            .as_deref()
            .and_then(|v| v.parse::<f64>().ok())
            .is_none_or(|max| parsed <= max)
    {
        return Err(invalid());
    }
    Ok(Value::Number(
        serde_json::Number::from_f64(parsed).ok_or_else(invalid)?,
    ))
}
pub fn normalize(
    fields: &[Field],
    changes: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, CatalogError> {
    if changes.is_empty() || changes.len() > 512 {
        return Err(invalid());
    }
    let mut result = BTreeMap::new();
    for (path, value) in changes {
        if (path.starts_with("llm.")
            && (path.ends_with(".model")
                || path.ends_with(".provider")
                || path == "llm.default_provider"))
            || (path.starts_with("runtime.ai.") && path.ends_with("_model"))
            || path == "runtime.bridges.matcher_ai_provider"
        {
            return Err(CatalogError("Die Modellauswahl erfolgt automatisch. Manuelle Anbieter- oder Modell-Pins sind nicht änderbar."));
        }
        let field = fields
            .iter()
            .find(|field| &field.path == path && field.writable)
            .ok_or(CatalogError(
                "Dieses Feld ist nicht über das Dashboard änderbar.",
            ))?;
        let normalized = if value.is_null() {
            if !field.nullable {
                return Err(invalid());
            }
            Value::Null
        } else {
            match field.kind {
                Kind::Boolean if value.is_boolean() => value.clone(),
                Kind::Integer => integer(field, value)?,
                Kind::Number => number(field, value)?,
                Kind::String => text(field, value)?,
                Kind::Choice
                    if value
                        .as_str()
                        .is_some_and(|v| field.choices.iter().any(|choice| choice == v)) =>
                {
                    value.clone()
                }
                Kind::IntegerList | Kind::StringList => {
                    let array = value.as_array().ok_or_else(invalid)?;
                    if array.len() > 512 || field.length.is_some_and(|length| array.len() != length)
                    {
                        return Err(invalid());
                    }
                    Value::Array(
                        array
                            .iter()
                            .map(|entry| {
                                if field.kind == Kind::IntegerList {
                                    integer(field, entry)
                                } else {
                                    text(field, entry)
                                }
                            })
                            .collect::<Result<_, _>>()?,
                    )
                }
                _ => return Err(invalid()),
            }
        };
        if path == SUPPORTER_TIERS_PATH && !supporter_tiers_allowed(&normalized) {
            return Err(CatalogError(
                "Die Unterstützer-Stufen passen nicht: Anzahl, aufsteigende Reihenfolge und Beträge stehen im Hilfetext.",
            ));
        }
        result.insert(path.clone(), normalized);
    }
    Ok(result)
}

const SUPPORTER_TIERS_PATH: &str = "runtime.community.supporter_tiers_eur";

fn supporter_tiers_allowed(value: &Value) -> bool {
    match value.as_array() {
        None => value.is_null(),
        Some(entries) => entries
            .iter()
            .map(Value::as_u64)
            .collect::<Option<Vec<_>>>()
            .is_some_and(|tiers| rc::supporter_tiers_valid(&tiers)),
    }
}

fn toml_value(value: &Value) -> Result<toml_edit::Value, CatalogError> {
    Ok(match value {
        Value::Bool(v) => (*v).into(),
        Value::String(v) => v.clone().into(),
        Value::Number(v) if v.is_i64() => v.as_i64().ok_or_else(invalid)?.into(),
        Value::Number(v) if v.is_u64() => return Err(invalid()),
        Value::Number(v) => v
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(invalid)?
            .into(),
        Value::Array(v) => {
            let mut array = toml_edit::Array::new();
            for entry in v {
                array.push(toml_value(entry)?);
            }
            array.into()
        }
        _ => return Err(invalid()),
    })
}
fn edit_table(
    table: &mut dyn toml_edit::TableLike,
    parts: &[&str],
    value: &Value,
) -> Result<(), CatalogError> {
    let (key, rest) = parts.split_first().ok_or_else(invalid)?;
    if rest.is_empty() {
        if value.is_null() {
            table.remove(key);
        } else {
            let mut replacement = toml_value(value)?;
            if let Some(previous) = table.get(key).and_then(toml_edit::Item::as_value) {
                *replacement.decor_mut() = previous.decor().clone();
            }
            table.insert(key, toml_edit::Item::Value(replacement));
        }
        return Ok(());
    }
    if !table.contains_key(key) {
        if value.is_null() {
            return Ok(());
        }
        table.insert(
            key,
            toml_edit::Item::Value(toml_edit::InlineTable::new().into()),
        );
    }
    let remove_empty_parent = {
        let child = table
            .get_mut(key)
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(invalid)?;
        edit_table(child, rest, value)?;
        value.is_null() && child.is_empty()
    };
    if remove_empty_parent {
        table.remove(key);
    }
    Ok(())
}
pub fn apply(
    document: &mut toml_edit::DocumentMut,
    changes: &BTreeMap<String, Value>,
) -> Result<(), CatalogError> {
    for (path, value) in changes {
        let parts: Vec<_> = path.split('.').collect();
        if parts.first() == Some(&"accounts") {
            if parts.len() < 3 {
                return Err(invalid());
            }
            let id = parts[1].parse::<i64>().map_err(|_| invalid())?;
            let item = document.get_mut("accounts").ok_or_else(invalid)?;
            if let Some(array) = item.as_array_of_tables_mut() {
                let account = array
                    .iter_mut()
                    .find(|entry| entry.get("id").and_then(toml_edit::Item::as_integer) == Some(id))
                    .ok_or_else(invalid)?;
                edit_table(account, &parts[2..], value)?;
            } else if let Some(array) = item.as_array_mut() {
                let account = array
                    .iter_mut()
                    .filter_map(toml_edit::Value::as_inline_table_mut)
                    .find(|entry| {
                        entry.get("id").and_then(toml_edit::Value::as_integer) == Some(id)
                    })
                    .ok_or_else(invalid)?;
                edit_table(account, &parts[2..], value)?;
            } else {
                return Err(invalid());
            }
        } else {
            edit_table(document.as_table_mut(), &parts, value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn loads_with(path: &str, value: Value) -> Result<(), String> {
        let fields = crate::admin_settings::fields();
        let changes = BTreeMap::from([(path.to_owned(), value)]);
        let normalized =
            normalize(&fields, &changes).map_err(|err| format!("Katalog lehnt ab: {err}"))?;
        let mut document: toml_edit::DocumentMut = "schema_version = 1\n"
            .parse()
            .map_err(|err| format!("{err}"))?;
        apply(&mut document, &normalized).map_err(|err| format!("Anwenden: {err}"))?;
        crate::bot_config::BotConfig::parse(&document.to_string())
            .map(|_| ())
            .map_err(|err| format!("Laden lehnt ab: {err}"))
    }
    fn max_chars(field: &Field) -> Option<usize> {
        field.max.as_deref().and_then(|max| max.parse().ok())
    }
    fn catalog_probes(field: &Field) -> Vec<Value> {
        let bounds = [&field.min, &field.max].into_iter().flatten();
        match field.kind {
            Kind::String => max_chars(field)
                .map(|max| Value::String("é".repeat(max)))
                .into_iter()
                .collect(),
            Kind::Integer => bounds.map(|bound| Value::String(bound.clone())).collect(),
            Kind::IntegerList => bounds
                .map(|bound| {
                    Value::Array(vec![
                        Value::String(bound.clone());
                        field.length.unwrap_or(1)
                    ])
                })
                .collect(),
            _ => {
                let bound = |raw: &Option<String>, open: f64| {
                    raw.as_deref()
                        .and_then(|value| value.parse::<f64>().ok())
                        .unwrap_or(open)
                };
                vec![
                    json!(bound(&field.min, -1e15)),
                    json!(bound(&field.max, 1e15)),
                ]
            }
        }
    }
    #[test]
    fn jeder_wert_im_katalogbereich_wird_auch_geladen() {
        let mut rejected = Vec::new();
        for field in crate::admin_settings::fields().iter().filter(|field| {
            field.writable
                && matches!(
                    field.kind,
                    Kind::Integer | Kind::Number | Kind::IntegerList | Kind::String
                )
        }) {
            for probe in catalog_probes(field) {
                if let Err(reason) = loads_with(&field.path, probe.clone()) {
                    rejected.extend([format!("{} = {probe}: {reason}", field.path)]);
                }
            }
        }
        assert!(rejected.is_empty(), "{rejected:#?}");
    }
    const PROBE_MARKER: &str = "__grenzprobe__";
    fn loader_accepts_raw(field: &Field, literal: &str) -> bool {
        let marker = Value::String(PROBE_MARKER.into());
        let placeholder = match field.kind {
            Kind::IntegerList => Value::Array(vec![marker; field.length.unwrap_or(1)]),
            _ => marker,
        };
        let mut document: toml_edit::DocumentMut = match "schema_version = 1\n".parse() {
            Ok(document) => document,
            Err(error) => panic!("TOML-Grundgerüst: {error}"),
        };
        if let Err(error) = apply(
            &mut document,
            &BTreeMap::from([(field.path.clone(), placeholder)]),
        ) {
            panic!("{}: Platzhalter nicht schreibbar: {error}", field.path);
        }
        let quoted = format!("\"{PROBE_MARKER}\"");
        let text = document.to_string();
        assert!(text.contains(&quoted), "{}: Platzhalter fehlt", field.path);
        crate::bot_config::BotConfig::parse(&text.replace(&quoted, literal)).is_ok()
    }
    fn one_past_literals(field: &Field) -> Vec<String> {
        let integer = |raw: &Option<String>| raw.as_deref().and_then(|v| v.parse::<i128>().ok());
        let number = |raw: &Option<String>| raw.as_deref().and_then(|v| v.parse::<f64>().ok());
        match field.kind {
            Kind::Integer | Kind::IntegerList => [
                integer(&field.min).map(|min| min - 1),
                integer(&field.max).map(|max| max + 1),
            ]
            .into_iter()
            .flatten()
            .map(|value| value.to_string())
            .collect(),
            Kind::Number => [
                number(&field.min).map(|min| min - 1e-6),
                number(&field.max).map(|max| max + 1e-6),
            ]
            .into_iter()
            .flatten()
            .map(|value| format!("{value:?}"))
            .collect(),
            Kind::String => max_chars(field)
                .map(|max| format!("\"{}\"", "é".repeat(max + 1)))
                .into_iter()
                .collect(),
            _ => vec![],
        }
    }
    #[test]
    fn loader_lehnt_jeden_wert_hinter_der_katalog_grenze_ab() {
        let mut accepted = BTreeSet::new();
        let mut probed = 0;
        for field in crate::admin_settings::fields()
            .iter()
            .filter(|field| field.writable)
        {
            for literal in one_past_literals(field) {
                probed += 1;
                if loader_accepts_raw(field, &literal) {
                    accepted.insert(field.path.clone());
                }
            }
        }
        assert!(probed > 0);
        assert_eq!(accepted, BTreeSet::new());
    }
    #[test]
    fn textgrenze_gilt_im_katalog_wie_im_loader() {
        let fields = crate::admin_settings::fields();
        let field = fields
            .iter()
            .find(|field| field.path == "runtime.start.presence_status")
            .unwrap_or_else(|| panic!("presence_status fehlt im Katalog"));
        assert_eq!(max_chars(field), Some(rc::PRESENCE_STATUS_MAX_CHARS));
        let change = |chars: usize| {
            normalize(
                &fields,
                &BTreeMap::from([(field.path.clone(), json!("é".repeat(chars)))]),
            )
        };
        assert!(change(rc::PRESENCE_STATUS_MAX_CHARS).is_ok());
        assert!(change(rc::PRESENCE_STATUS_MAX_CHARS + 1).is_err());
    }
    #[test]
    fn katalog_und_loader_urteilen_bei_jedem_text_gleich() {
        let fields = crate::admin_settings::fields();
        let probes = [
            "",
            " ",
            "\u{a0}",
            "x",
            "!",
            "info",
            "dl_bot=debug",
            "[",
            "0",
            "123",
        ];
        let mut mismatched = Vec::new();
        let mut probed = 0;
        for field in fields
            .iter()
            .filter(|field| field.writable && field.kind == Kind::String)
        {
            for probe in probes {
                probed += 1;
                let catalog = normalize(
                    &fields,
                    &BTreeMap::from([(field.path.clone(), json!(probe))]),
                )
                .is_ok();
                let loader = loader_accepts_raw(field, &json!(probe).to_string());
                if catalog != loader {
                    mismatched.push(format!(
                        "{} = {probe:?}: Katalog {catalog}, Loader {loader}",
                        field.path
                    ));
                }
            }
        }
        assert!(probed > 0);
        assert!(mismatched.is_empty(), "{mismatched:#?}");
    }
    #[test]
    fn grenzen_stehen_nur_an_aenderbaren_katalogzeilen() {
        let fields = crate::admin_settings::fields();
        for field in fields.iter().filter(|field| !field.writable) {
            assert_eq!(
                (&field.min, &field.max),
                (&None, &None),
                "{}: Grenze an geschützter Zeile",
                field.path
            );
        }
        let writable = |path: &str| {
            fields
                .iter()
                .any(|field| field.path == path && field.writable)
        };
        let table_paths = LOADER_BOUNDS
            .iter()
            .flat_map(|(paths, _)| paths.iter())
            .chain(
                LOADER_NUMBER_BOUNDS
                    .iter()
                    .flat_map(|(paths, _)| paths.iter()),
            )
            .chain(
                LOADER_TEXT_MAX_CHARS
                    .iter()
                    .flat_map(|(paths, _)| paths.iter()),
            )
            .chain(LOADER_TEXT_RULES.iter().flat_map(|(paths, _)| paths.iter()));
        for path in table_paths {
            assert!(writable(path), "{path}: Grenze ohne änderbare Katalogzeile");
        }
        let suffixes = LLM_USE_CASE_BOUNDS
            .iter()
            .map(|(suffix, _)| *suffix)
            .chain(LLM_USE_CASE_NUMBER_BOUNDS.iter().map(|(suffix, _)| *suffix));
        for suffix in suffixes {
            assert!(
                fields.iter().any(|field| field.writable
                    && field.path.starts_with(LLM_USE_CASE_PREFIX)
                    && field.path.ends_with(suffix)),
                "{suffix}: Grenze ohne änderbare Katalogzeile"
            );
        }
    }
    #[test]
    fn deutsche_zahlen_fuer_grenzen() {
        for (value, text) in [
            (0, "0"),
            (120, "120"),
            (1000, "1.000"),
            (86_400, "86.400"),
            (99_999_999, "99.999.999"),
            (i64::MAX as u64, "9.223.372.036.854.775.807"),
        ] {
            assert_eq!(german_number(value), text);
        }
        assert_eq!(german_decimal(0.0), "0");
        assert_eq!(german_decimal(1.0), "1");
        assert_eq!(german_decimal(0.001), "0,001");
    }
    #[test]
    fn katalogtexte_nennen_grenzen_nur_als_platzhalter() {
        for field in crate::admin_settings::fields() {
            for text in [&field.label, &field.help] {
                assert!(
                    !text.contains('{') && !text.contains('}'),
                    "{}: Platzhalter ohne Wert: {text}",
                    field.path
                );
            }
        }
        for line in crate::admin_settings::SPEC
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let columns: Vec<_> = line.split('\t').collect();
            for text in [columns[3], columns[5]] {
                assert!(
                    !text.chars().any(|c| c.is_ascii_digit()),
                    "{}: Zahl im Text statt Platzhalter: {text}",
                    columns[0]
                );
            }
        }
    }
    fn supporter_field(path: &str) -> Field {
        crate::admin_settings::fields()
            .into_iter()
            .find(|field| field.path == path)
            .unwrap_or_else(|| panic!("{path} fehlt im Katalog"))
    }
    #[test]
    fn unterstuetzer_hilfetexte_und_grenzen_folgen_den_konstanten() {
        let expected = [
            (
                "runtime.community.supporter_role_sync_max_revokes",
                format!(
                    "Standard ist {}, höchstens {}. Bei {} wird niemandem",
                    rc::SUPPORTER_DEFAULT_MAX_REVOKES,
                    german_number(*rc::SUPPORTER_MAX_REVOKES_RANGE.end()),
                    german_number(*rc::SUPPORTER_MAX_REVOKES_RANGE.start())
                ),
                rc::SUPPORTER_MAX_REVOKES_RANGE.start().to_string(),
                rc::SUPPORTER_MAX_REVOKES_RANGE.end().to_string(),
            ),
            (
                "runtime.community.supporter_sync_interval_seconds",
                format!(
                    "Standard ist {}, mindestens {}, höchstens {}.",
                    rc::SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS,
                    german_number(*rc::SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.start()),
                    german_number(*rc::SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.end())
                ),
                rc::SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.start().to_string(),
                rc::SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.end().to_string(),
            ),
            (
                "runtime.community.supporter_funding_goal_cents",
                format!(
                    "Standard ist {}, also {} €. Erlaubt sind {} bis {}.",
                    rc::SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS,
                    rc::SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS / 100,
                    german_number(rc::SUPPORTER_FUNDING_GOAL_CENTS_RANGE.start().unsigned_abs()),
                    german_number(rc::SUPPORTER_FUNDING_GOAL_CENTS_RANGE.end().unsigned_abs())
                ),
                rc::SUPPORTER_FUNDING_GOAL_CENTS_RANGE.start().to_string(),
                rc::SUPPORTER_FUNDING_GOAL_CENTS_RANGE.end().to_string(),
            ),
            (
                "runtime.community.supporter_tiers_eur",
                format!(
                    "Standard ist {}. Mindestens eine und höchstens {} Stufen, aufsteigend, jede mindestens {} € und höchstens {} €.",
                    rc::SUPPORTER_DEFAULT_TIERS_EUR
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    rc::SUPPORTER_MAX_TIERS,
                    german_number(rc::SUPPORTER_TIER_MIN_EUR),
                    german_number(rc::SUPPORTER_TIER_MAX_EUR)
                ),
                rc::SUPPORTER_TIER_MIN_EUR.to_string(),
                rc::SUPPORTER_TIER_MAX_EUR.to_string(),
            ),
        ];
        for (path, help, min, max) in expected {
            let field = supporter_field(path);
            assert!(field.writable, "{path}");
            assert!(field.help.contains(&help), "{path}: {}", field.help);
            assert_eq!(field.min.as_deref(), Some(min.as_str()), "{path}");
            assert_eq!(field.max.as_deref(), Some(max.as_str()), "{path}");
        }
    }
    #[test]
    fn unterstuetzer_stufen_ausserhalb_der_grenzen_werden_abgelehnt() {
        let fields = vec![supporter_field("runtime.community.supporter_tiers_eur")];
        let change = |tiers: Value| {
            normalize(
                &fields,
                &BTreeMap::from([("runtime.community.supporter_tiers_eur".into(), tiers)]),
            )
        };
        let text = |eur: u64| eur.to_string();
        let rejected = |tiers: Value| {
            change(tiers)
                .err()
                .map(|error| error.0)
                .is_some_and(|message| message.starts_with("Die Unterstützer-Stufen passen nicht"))
        };
        assert!(rejected(json!([])));
        assert!(rejected(json!([text(10), text(5)])));
        assert!(rejected(json!([text(5), text(5)])));
        assert!(rejected(json!((0..=rc::SUPPORTER_MAX_TIERS as u64)
            .map(|step| text(rc::SUPPORTER_TIER_MIN_EUR + step))
            .collect::<Vec<_>>())));
        assert!(change(Value::Null).is_ok());
        assert!(change(json!([text(rc::SUPPORTER_TIER_MIN_EUR - 1)])).is_err());
        assert!(change(json!([text(rc::SUPPORTER_TIER_MAX_EUR + 1)])).is_err());
        assert!(change(json!([
            text(rc::SUPPORTER_TIER_MIN_EUR),
            text(rc::SUPPORTER_TIER_MAX_EUR)
        ]))
        .is_ok());
    }
    const SPEC: &str = "role\tOption<u64>\tTest\tRolle\tedit\tTest\nprotected\tString\tTest\tGeschützt\tprotected\tInfisical\n";
    #[test]
    fn exact_integers_and_secret_boundary() {
        let fields = fields(SPEC, &[]);
        let catalog = build(
            &json!({"role": 1547199955133927464u64,"protected":"not-for-browser"}),
            fields.clone(),
        );
        assert_eq!(catalog.values["role"], "1547199955133927464");
        assert!(!catalog.values.contains_key("protected"));
        assert!(normalize(
            &fields,
            &BTreeMap::from([("role".into(), json!(1547199955133927464u64))])
        )
        .is_err());
        assert!(normalize(&fields, &BTreeMap::from([("role".into(), json!("1e18"))])).is_err());
        assert!(normalize(
            &fields,
            &BTreeMap::from([("protected".into(), json!("other"))])
        )
        .is_err());
        assert!(normalize(&fields, &BTreeMap::from([("unknown".into(), json!(true))])).is_err());
        assert_eq!(
            normalize(&fields, &catalog.values).expect("gültig")["role"],
            json!(1547199955133927464u64)
        );
    }
    #[test]
    fn nested_tables_inline_tables_and_nullable_removal() {
        for raw in [
            "[runtime.ai]\n# bleibt\nmodel = 'old' # Kommentar\n",
            "runtime = { ai = { model = 'old' } }\n",
            "",
        ] {
            let mut document: toml_edit::DocumentMut = raw.parse().expect("TOML");
            apply(
                &mut document,
                &BTreeMap::from([("runtime.ai.model".into(), json!("new"))]),
            )
            .expect("setzen");
            let parsed: Value = toml::from_str(&document.to_string()).expect("TOML");
            assert_eq!(parsed["runtime"]["ai"]["model"], "new");
            apply(
                &mut document,
                &BTreeMap::from([("runtime.ai.model".into(), Value::Null)]),
            )
            .expect("entfernen");
            let parsed: Value = toml::from_str(&document.to_string()).expect("TOML");
            assert!(parsed["runtime"]["ai"].get("model").is_none());
        }
    }

    #[test]
    fn whole_json_numbers_for_float_fields_roundtrip_as_toml_floats() {
        let spec = "llm.use_cases.test.temperature\tOption<f64>\tTest\tTemperatur\tedit\tTest\n\
runtime.bridges.twitch_timeout_seconds\tOption<f64>\tTest\tTimeout\tedit\tTest\n\
runtime.moderation.analyze_flag_threshold\tOption<f64>\tTest\tSchwelle\tedit\tTest\n";
        let fields = fields(spec, &[]);
        let changes = BTreeMap::from([
            ("llm.use_cases.test.temperature".into(), json!(1)),
            ("runtime.bridges.twitch_timeout_seconds".into(), json!(1)),
            ("runtime.moderation.analyze_flag_threshold".into(), json!(1)),
        ]);
        let normalized = normalize(&fields, &changes).expect("Ganzzahlen für f64 gültig");
        assert!(normalized.values().all(|value| value.as_f64() == Some(1.0)
            && value.as_number().is_some_and(serde_json::Number::is_f64)));
        let mut document: toml_edit::DocumentMut = "".parse().expect("leeres TOML");
        apply(&mut document, &normalized).expect("TOML-Schreibpfad");
        let parsed: toml::Value =
            toml::from_str(&document.to_string()).expect("geschriebenes TOML");
        assert_eq!(
            parsed["llm"]["use_cases"]["test"]["temperature"].as_float(),
            Some(1.0)
        );
        assert_eq!(
            parsed["runtime"]["bridges"]["twitch_timeout_seconds"].as_float(),
            Some(1.0)
        );
        assert_eq!(
            parsed["runtime"]["moderation"]["analyze_flag_threshold"].as_float(),
            Some(1.0)
        );
    }

    #[test]
    fn number_rejects_imprecise_json_integers_and_u64_toml_overflow() {
        let fields = fields(
            "threshold\tf64\tTest\tSchwelle\tedit\tTest\nrole\tu64\tTest\tRolle\tedit\tTest\n",
            &[],
        );
        let change = |path: &str, value| BTreeMap::from([(path.to_owned(), value)]);
        assert!(normalize(&fields, &change("threshold", json!((1u64 << 53) - 1))).is_ok());
        assert!(normalize(&fields, &change("threshold", json!(1u64 << 53))).is_err());
        assert!(normalize(&fields, &change("threshold", json!(u64::MAX))).is_err());
        assert_eq!(
            normalize(&fields, &change("role", json!(i64::MAX.to_string()))).expect("i64-Grenze")
                ["role"],
            json!(i64::MAX)
        );
        assert!(normalize(
            &fields,
            &change("role", json!((i64::MAX as u64 + 1).to_string()))
        )
        .is_err());
        assert!(toml_value(&json!(u64::MAX)).is_err());
    }
}
