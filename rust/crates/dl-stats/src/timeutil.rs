//! Zeit-Helfer mit Python-datetime-Semantik (public_stats.py).
//!
//! Das Original arbeitet durchgehend mit NAIVEN Zeitwerten
//! (`datetime.now()`, `fromisoformat`, `isoformat`) — die SQLite-DB speicherte
//! ISO-Strings ohne Zeitzone. Central-Postgres hält diese Wandzeit als UTC,
//! deshalb werden Buckets aus `naive_utc()` gebildet.

#[cfg(test)]
use chrono::NaiveDate;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, SecondsFormat, Timelike, Utc};

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
#[cfg(test)]
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

pub fn isoformat_utc(dt: DateTime<Utc>) -> String {
    let format = if dt.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    dt.to_rfc3339_opts(format, true)
}

pub fn to_iso(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(isoformat_utc)
}

pub fn utc_naive(dt: &DateTime<Utc>) -> NaiveDateTime {
    dt.naive_utc()
}

#[cfg(test)]
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
pub(crate) fn sqlite_week_label(dt: &NaiveDateTime) -> String {
    let doy0 = i64::from(dt.ordinal0());
    let jan1 = dt
        .date()
        .with_ordinal(1)
        .expect("Jahr hat immer einen ersten Tag");
    let first_monday_doy0 = (7 - i64::from(jan1.weekday().num_days_from_monday())) % 7;
    let week = (doy0 - first_monday_doy0 + 7) / 7;
    format!("{}-{week:02}", dt.format("%Y"))
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
        assert_eq!(
            isoformat_keeping_offset("2026-06-10T20:15:30+00:00", dt).as_str(),
            "2026-06-10T20:15:30+00:00"
        );
    }

    #[test]
    fn wochentag_montag_null() {
        // 2026-06-10 ist ein Mittwoch → 2
        let dt = parse_iso("2026-06-10T00:00:00").expect("parse");
        assert_eq!(weekday_mon0(&dt), 2);
    }

    #[test]
    fn sqlite_woche_montag_basiert_mit_woche_null() {
        assert_eq!(
            sqlite_week_label(&parse_iso("2026-01-04T23:30:00").expect("parse")),
            "2026-00"
        );
        assert_eq!(
            sqlite_week_label(&parse_iso("2026-01-05T00:30:00").expect("parse")),
            "2026-01"
        );
        assert_eq!(
            sqlite_week_label(&parse_iso("2018-12-31T00:00:00").expect("parse")),
            "2018-53"
        );
    }

    #[test]
    fn to_iso_grenzfaelle() {
        assert_eq!(to_iso(None), None);
        let dt = DateTime::parse_from_rfc3339("2026-06-10T20:15:30+00:00")
            .expect("parse")
            .with_timezone(&Utc);
        assert_eq!(to_iso(Some(dt)).as_deref(), Some("2026-06-10T20:15:30Z"));
    }
}
