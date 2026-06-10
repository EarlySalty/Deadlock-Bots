//! Kleine Helfer mit Python-äquivalenter Semantik (tierlist_public.py).

use serde_json::Value;

/// Rundet auf 2 Nachkommastellen exakt wie Pythons `round()`.
pub fn py_round2(x: f64) -> f64 {
    py_round_decimal(x, 2)
}

/// Rundet auf 4 Nachkommastellen (Winrate-Normalisierung beim Refresh).
pub fn py_round4(x: f64) -> f64 {
    py_round_decimal(x, 4)
}

/// Pythons `round(x, nd)`: Half-to-even auf der EXAKTEN Dezimaldarstellung
/// des Doubles — nicht auf `x * 10^nd` (der Multiplikationsfehler kippt
/// Grenzfälle wie 50.365 in die falsche Richtung, Python rundet dezimal
/// korrekt). Wir formatieren den exakten Wert mit 35 Nachkommastellen
/// (jede Binärbruch-Expansion ist bis dahin eindeutig entschieden) und
/// runden auf der Ziffernfolge.
fn py_round_decimal(x: f64, nd: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let neg = x.is_sign_negative();
    let s = format!("{:.35}", x.abs());
    let Some((int_part, frac_part)) = s.split_once('.') else {
        return x;
    };
    if nd >= frac_part.len() {
        return x;
    }

    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes())
        .map(|b| b - b'0')
        .collect();
    let mut int_len = int_part.len();
    let cut = int_len + nd; // Index der ersten weggerundeten Ziffer

    let first_dropped = digits[cut];
    let round_up = match first_dropped.cmp(&5) {
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => {
            if digits[cut + 1..].iter().any(|d| *d != 0) {
                true
            } else {
                // exakter Tie → zur geraden Ziffer
                digits[cut - 1] % 2 == 1
            }
        }
    };
    digits.truncate(cut);
    if round_up {
        let mut i = cut;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                int_len += 1;
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }

    let mut out = String::with_capacity(digits.len() + 2);
    if neg {
        out.push('-');
    }
    for (i, d) in digits.iter().enumerate() {
        if i == int_len {
            out.push('.');
        }
        out.push((b'0' + d) as char);
    }
    out.parse::<f64>().unwrap_or(x)
}

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Wie Python `_coerce_int`: None/""/false → default; float wird abgeschnitten;
/// Strings nur als ganze Zahl.
pub fn coerce_int(value: Option<&Value>, default: Option<i64>) -> Option<i64> {
    let value = match value {
        None | Some(Value::Null) => return default,
        Some(v) => v,
    };
    match value {
        Value::Bool(false) => default,
        Value::Bool(true) => Some(1),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .or(default),
        Value::String(s) if s.is_empty() => default,
        Value::String(s) => s.trim().parse::<i64>().ok().or(default),
        _ => default,
    }
}

/// Wie Python `_coerce_float`.
pub fn coerce_float(value: Option<&Value>, default: Option<f64>) -> Option<f64> {
    let value = match value {
        None | Some(Value::Null) => return default,
        Some(v) => v,
    };
    match value {
        Value::Bool(false) => default,
        Value::Bool(true) => Some(1.0),
        Value::Number(n) => n.as_f64().or(default),
        Value::String(s) if s.is_empty() => default,
        Value::String(s) => s.trim().parse::<f64>().ok().or(default),
        _ => default,
    }
}

/// Wie Python `_coerce_bool`.
pub fn coerce_bool(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(default),
        Some(Value::String(s)) => match s.trim().to_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        _ => default,
    }
}

/// Wie Python `_parse_unix_or_iso`: Unix-Timestamp (Zahl/Ziffern-String) oder
/// ISO-Datum; naive Zeiten gelten als UTC. None/""/false → None.
pub fn parse_unix_or_iso(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    match value {
        Value::Null | Value::Bool(false) => None,
        Value::Bool(true) => Some(1),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => {
            let text = s.trim();
            if text.is_empty() {
                return None;
            }
            if text.chars().all(|c| c.is_ascii_digit()) {
                return text.parse::<i64>().ok();
            }
            parse_iso_to_unix(text)
        }
        _ => None,
    }
}

fn parse_iso_to_unix(text: &str) -> Option<i64> {
    let text = if let Some(stripped) = text.strip_suffix('Z') {
        format!("{stripped}+00:00")
    } else {
        text.to_string()
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&text) {
        return Some(dt.timestamp());
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&text, fmt) {
            return Some(naive.and_utc().timestamp());
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_unix_oder_iso() {
        assert_eq!(parse_unix_or_iso(Some(&json!(1234))), Some(1234));
        assert_eq!(parse_unix_or_iso(Some(&json!("1234"))), Some(1234));
        assert_eq!(parse_unix_or_iso(Some(&json!(""))), None);
        assert_eq!(parse_unix_or_iso(Some(&json!(null))), None);
        assert_eq!(parse_unix_or_iso(Some(&json!(false))), None);
        // 2026-01-01T00:00:00Z
        assert_eq!(
            parse_unix_or_iso(Some(&json!("2026-01-01T00:00:00Z"))),
            Some(1767225600)
        );
        assert_eq!(
            parse_unix_or_iso(Some(&json!("2026-01-01"))),
            Some(1767225600)
        );
    }

    #[test]
    fn runden_wie_python() {
        // Referenzwerte aus CPython: round(x, 2)
        assert_eq!(py_round2(53.119), 53.12);
        assert_eq!(py_round2(52.0), 52.0);
        assert_eq!(py_round2(0.125), 0.12); // exakter Tie → gerade
        assert_eq!(py_round2(55.355), 55.35); // binär knapp UNTER dem Tie
        assert_eq!(py_round2(50.365), 50.37); // binär knapp ÜBER dem Tie
        assert_eq!(py_round2(50.875), 50.88);
        assert_eq!(py_round2(48.095), 48.09);
        assert_eq!(py_round2(2.675), 2.67);
        assert_eq!(py_round2(-55.355), -55.35);
        assert_eq!(py_round2(99.995), 100.0); // Carry über alle Stellen
        assert_eq!(py_round4(55.12345), 55.1234);
        assert_eq!(py_round4(0.00005), 0.0001);
    }

    #[test]
    fn coerce_grenzfaelle() {
        assert_eq!(coerce_int(Some(&json!("5")), None), Some(5));
        assert_eq!(coerce_int(Some(&json!("5.7")), None), None);
        assert_eq!(coerce_int(Some(&json!(5.7)), None), Some(5));
        assert_eq!(coerce_int(Some(&json!(false)), Some(9)), Some(9));
        assert!(coerce_bool(Some(&json!("yes")), false));
        assert!(!coerce_bool(Some(&json!(0)), true));
        assert!(coerce_bool(None, true));
    }
}
