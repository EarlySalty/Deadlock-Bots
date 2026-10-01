//! Versionierter Admin-Vertrag. Nur Einträge im geprüften Katalog sind schreibbar.
//! Ganzzahlen laufen als Dezimalstrings über JSON, niemals als JavaScript-Number.
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
            let mut bounds = match integer_type {
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
            if path.starts_with("llm.use_cases.") {
                if path.ends_with(".max_output_tokens") {
                    bounds = Some((1, 32_768));
                }
                if path.ends_with(".temperature") {
                    bounds = Some((0, 2));
                }
                if path.ends_with(".request_timeout_seconds") {
                    bounds = Some((1, 110));
                }
            }
            match path.as_str() {
                "concierge.timeout_seconds" => bounds = Some((1, 110)),
                "tempvoice.empty_lane_grace_seconds" => bounds = Some((1, 86_400)),
                "runtime.bridges.matcher_max_ai_per_scan" => bounds = Some((0, 1000)),
                "runtime.bridges.matcher_scan_interval_hours" => bounds = Some((1, 8760)),
                "runtime.ai.brain_max_question_len" => bounds = Some((1, 65_536)),
                _ => {}
            }
            result.push(Field {
                path,
                group: replace(columns[2]),
                label: replace(columns[3]),
                kind,
                nullable,
                writable: columns[4] == "edit",
                restart_required: true,
                help: replace(columns[5]),
                min: bounds.map(|(min, _)| min.to_string()),
                max: bounds.map(|(_, max)| max.to_string()),
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
    // Keine Rundung, Exponenten, Dezimalbrüche oder mehrdeutigen Schreibweisen.
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
fn text(value: &Value) -> Result<Value, CatalogError> {
    let raw = value.as_str().ok_or_else(invalid)?;
    if raw.len() > 2048 || raw.chars().any(char::is_control) {
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
                Kind::String => text(value)?,
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
                                    text(entry)
                                }
                            })
                            .collect::<Result<_, _>>()?,
                    )
                }
                _ => return Err(invalid()),
            }
        };
        result.insert(path.clone(), normalized);
    }
    Ok(result)
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
