//! Payload-Parsing mit Python-identischen Fehlertexten (ValueError-Strings
//! gehen 1:1 als `bad_request`-Message in die Antwort).

use serde_json::{Map, Value};

use crate::port::ViewSpec;

pub type ParseResult<T> = Result<T, String>;

/// `_parse_positive_payload_int`: int oder Ziffern-String, > 0; bool ist Fehler.
pub fn positive_int(payload: &Map<String, Value>, key: &str) -> ParseResult<u64> {
    let raw = payload.get(key);
    let value = match raw {
        Some(Value::Bool(_)) | None => None,
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                trimmed.parse::<i64>().ok()
            } else {
                None
            }
        }
        _ => None,
    };
    match value {
        Some(v) if v > 0 => Ok(v as u64),
        _ => Err(format!("{key} must be a positive integer")),
    }
}

/// `_parse_optional_content`: None/leer → None; nur Strings; ≤ 2000.
pub fn optional_content(payload: &Map<String, Value>, key: &str) -> ParseResult<Option<String>> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let content = s.trim();
            if content.is_empty() {
                Ok(None)
            } else if content.chars().count() > 2000 {
                Err(format!("{key} exceeds Discord limit (2000)"))
            } else {
                Ok(Some(content.to_string()))
            }
        }
        Some(_) => Err(format!("{key} must be a string")),
    }
}

/// Pflicht-Content für send-message/send-dm.
pub fn required_content(payload: &Map<String, Value>) -> ParseResult<String> {
    let content = payload
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err("content is required".to_string());
    }
    if content.chars().count() > 2000 {
        return Err("content exceeds Discord limit (2000)".to_string());
    }
    Ok(content)
}

/// `_parse_embed_dict`: muss ein JSON-Objekt sein (Tiefenprüfung macht Discord).
pub fn embed_dict(payload: &Map<String, Value>) -> ParseResult<Map<String, Value>> {
    match payload.get("embed") {
        Some(Value::Object(obj)) => Ok(obj.clone()),
        _ => Err("embed must be a JSON object".to_string()),
    }
}

/// `_parse_int_id_list`: Liste positiver Ints (oder Ziffern-Strings), dedupliziert.
pub fn id_list(payload: &Map<String, Value>, key: &str) -> ParseResult<Vec<u64>> {
    let raw = match payload.get(key) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(format!("{key} must be a list of positive integers")),
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in raw {
        let value = match item {
            Value::Bool(_) => None,
            Value::Number(n) => n.as_i64(),
            Value::String(s) => {
                let trimmed = s.trim();
                if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                    trimmed.parse::<i64>().ok()
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(value) = value.filter(|v| *v > 0) else {
            return Err(format!("{key} must be a list of positive integers"));
        };
        if seen.insert(value) {
            out.push(value as u64);
        }
    }
    Ok(out)
}

/// `_parse_view_spec`: type ∈ {twitch_live_tracking, link_button}.
pub fn view_spec(payload: &Map<String, Value>) -> ParseResult<Option<ViewSpec>> {
    let raw = match payload.get("view_spec") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(obj)) => obj,
        Some(_) => return Err("view_spec must be a JSON object".to_string()),
    };
    let view_type = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let get = |key: &str| -> String {
        raw.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    match view_type {
        "link_button" => {
            let label = get("label");
            if label.is_empty() {
                return Err("view_spec.label is required".to_string());
            }
            if label.chars().count() > 80 {
                return Err("view_spec.label exceeds Discord limit (80)".to_string());
            }
            let url = get("url");
            if url.is_empty() {
                return Err("view_spec.url is required".to_string());
            }
            let valid_scheme = url.starts_with("http://") || url.starts_with("https://");
            let has_host = url
                .split_once("://")
                .map(|(_, rest)| !rest.is_empty() && !rest.starts_with('/'))
                .unwrap_or(false);
            if !valid_scheme || !has_host {
                return Err("view_spec.url must use http or https".to_string());
            }
            Ok(Some(ViewSpec::LinkButton { label, url }))
        }
        "twitch_live_tracking" => Ok(Some(ViewSpec::TwitchLiveTracking {
            streamer_login: get("streamer_login"),
            referral_url: get("referral_url"),
            tracking_token: get("tracking_token"),
            button_label: get("button_label"),
        })),
        _ => Err("view_spec.type is invalid".to_string()),
    }
}

/// `_extract_idempotency_key`: Header und/oder Body, Konsistenz-Pflicht, ≤128.
pub fn idempotency_key(header: Option<&str>, payload: &Map<String, Value>) -> ParseResult<String> {
    let header_key = header.unwrap_or_default().trim().to_string();
    let body_key = payload
        .get("idempotency_key")
        .map(|v| match v {
            Value::String(s) => s.trim().to_string(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    if !header_key.is_empty() && !body_key.is_empty() && header_key != body_key {
        return Err("header/body idempotency key mismatch".to_string());
    }
    let key = if header_key.is_empty() {
        body_key
    } else {
        header_key
    };
    if key.is_empty() {
        return Err("idempotency_key is required".to_string());
    }
    if key.chars().count() > 128 {
        return Err("idempotency_key is too long".to_string());
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("objekt")
    }

    #[test]
    fn positive_int_grenzfaelle() {
        assert_eq!(positive_int(&obj(json!({"x": 5})), "x"), Ok(5));
        assert_eq!(positive_int(&obj(json!({"x": "7"})), "x"), Ok(7));
        assert!(positive_int(&obj(json!({"x": 0})), "x").is_err());
        assert!(positive_int(&obj(json!({"x": true})), "x").is_err());
        assert!(positive_int(&obj(json!({"x": "5.5"})), "x").is_err());
        assert!(positive_int(&obj(json!({})), "x").is_err());
    }

    #[test]
    fn id_list_dedupliziert_und_validiert() {
        assert_eq!(
            id_list(&obj(json!({"ids": [3, "5", 3]})), "ids"),
            Ok(vec![3, 5])
        );
        assert_eq!(id_list(&obj(json!({})), "ids"), Ok(vec![]));
        assert!(id_list(&obj(json!({"ids": [0]})), "ids").is_err());
        assert!(id_list(&obj(json!({"ids": "nope"})), "ids").is_err());
    }

    #[test]
    fn view_spec_validierung() {
        let spec = view_spec(&obj(json!({"view_spec": {
            "type": "link_button", "label": "Öffnen", "url": "https://example.com/x"
        }})))
        .expect("ok")
        .expect("some");
        assert!(matches!(spec, ViewSpec::LinkButton { .. }));

        assert!(view_spec(&obj(json!({"view_spec": {"type": "evil"}}))).is_err());
        assert!(view_spec(&obj(json!({"view_spec": {
            "type": "link_button", "label": "x", "url": "javascript:alert(1)"
        }})))
        .is_err());
        assert_eq!(view_spec(&obj(json!({}))).expect("ok"), None);
    }

    #[test]
    fn idempotency_key_regeln() {
        let empty = obj(json!({}));
        assert!(idempotency_key(None, &empty).is_err());
        assert_eq!(idempotency_key(Some("k-1"), &empty), Ok("k-1".to_string()));
        let body = obj(json!({"idempotency_key": "k-1"}));
        assert_eq!(idempotency_key(Some("k-1"), &body), Ok("k-1".to_string()));
        assert!(idempotency_key(Some("anders"), &body).is_err());
    }
}
