//! Zeit-Helfer mit Python-datetime-Semantik (public_stats.py).
//!
//! Das Original arbeitet durchgehend mit NAIVEN Lokalzeiten
//! (`datetime.now()`, `fromisoformat`, `isoformat`) — die DB speichert
//! ISO-Strings ohne Zeitzone. Wir spiegeln das mit `chrono::NaiveDateTime`
//! in Lokalzeit.

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};

pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local()
}

/// `datetime.isoformat()`: Mikrosekunden nur wenn != 0.
pub fn isoformat(dt: NaiveDateTime) -> String {
    if dt.and_utc().timestamp_subsec_micros() == 0 {
        dt.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    }
}

/// `datetime.fromisoformat(text.replace("Z", "+00:00"))` — tolerant gegen
/// T- oder Leerzeichen-Separator, optionale Sekundenbruchteile, optionalen
/// Offset (der Offset wird wie in Python NICHT konvertiert, nur getragen —
/// fürs Bucketing zählt die naive Komponente).
pub fn parse_iso(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let cleaned = text.replace('Z', "+00:00");
    // Offset abschneiden (Python behält ihn als tzinfo; weekday()/hour
    // beziehen sich auf die naive Komponente — identisch zu diesem Schnitt).
    let naive_part = match cleaned.char_indices().rev().find(|(i, c)| {
        (*c == '+' || *c == '-') && *i >= 10 // Minus im Datum (Y-M-D) ignorieren
    }) {
        Some((i, _)) => &cleaned[..i],
        None => cleaned.as_str(),
    };
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(naive_part, fmt) {
            return Some(dt);
        }
    }
    NaiveDate::parse_from_str(naive_part, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Python `_to_iso`: SQLite-Wert (TEXT/Zahl/NULL) → normalisierter
/// ISO-String oder null; Unparsebares wird unverändert durchgereicht.
pub fn to_iso(value: rusqlite::types::Value) -> Option<String> {
    use rusqlite::types::Value;
    match value {
        Value::Null => None,
        Value::Integer(n) => Some(isoformat(local_from_timestamp(n as f64)?)),
        Value::Real(f) => Some(isoformat(local_from_timestamp(f)?)),
        Value::Text(s) => {
            if s.is_empty() {
                return None;
            }
            match parse_iso(&s) {
                Some(dt) => Some(isoformat_keeping_offset(&s, dt)),
                None => Some(s),
            }
        }
        Value::Blob(b) => Some(String::from_utf8_lossy(&b).to_string()),
    }
}

/// `datetime.fromtimestamp(x)` — Lokalzeit.
fn local_from_timestamp(ts: f64) -> Option<NaiveDateTime> {
    let secs = ts.floor() as i64;
    let micros = ((ts - secs as f64) * 1_000_000.0).round() as u32;
    Local
        .timestamp_opt(secs, micros * 1000)
        .single()
        .map(|dt| dt.naive_local())
}

/// Python gibt bei vorhandenem Offset diesen im isoformat wieder aus.
/// Wir hängen einen im Original-String vorhandenen Offset wieder an.
fn isoformat_keeping_offset(original: &str, naive: NaiveDateTime) -> String {
    let base = isoformat(naive);
    let cleaned = original.trim().replace('Z', "+00:00");
    if let Some(idx) = cleaned
        .char_indices()
        .rev()
        .find(|(i, c)| (*c == '+' || *c == '-') && *i >= 10)
        .map(|(i, _)| i)
    {
        format!("{base}{}", &cleaned[idx..])
    } else {
        base
    }
}

/// Python `weekday()`: Montag = 0.
pub fn weekday_mon0(dt: &NaiveDateTime) -> usize {
    dt.weekday().num_days_from_monday() as usize
}

pub fn hour(dt: &NaiveDateTime) -> usize {
    dt.hour() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_und_format_roundtrip() {
        let dt = parse_iso("2026-06-10T20:15:30.123456").expect("parse");
        assert_eq!(isoformat(dt), "2026-06-10T20:15:30.123456");
        // Leerzeichen-Separator wird wie Python 3.11 fromisoformat akzeptiert
        // und beim Ausgeben zu T normalisiert
        let dt = parse_iso("2026-06-10 20:15:30").expect("parse");
        assert_eq!(isoformat(dt), "2026-06-10T20:15:30");
        // Offset wird beim Bucketing ignoriert, bei to_iso aber erhalten
        let v = rusqlite::types::Value::Text("2026-06-10T20:15:30+00:00".into());
        assert_eq!(to_iso(v).as_deref(), Some("2026-06-10T20:15:30+00:00"));
    }

    #[test]
    fn wochentag_montag_null() {
        // 2026-06-10 ist ein Mittwoch → 2
        let dt = parse_iso("2026-06-10T00:00:00").expect("parse");
        assert_eq!(weekday_mon0(&dt), 2);
    }

    #[test]
    fn to_iso_grenzfaelle() {
        use rusqlite::types::Value;
        assert_eq!(to_iso(Value::Null), None);
        assert_eq!(to_iso(Value::Text(String::new())), None);
        assert_eq!(
            to_iso(Value::Text("kein-datum".into())).as_deref(),
            Some("kein-datum")
        );
    }
}
