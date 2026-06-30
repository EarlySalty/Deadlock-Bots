use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::{Number, Value};

const MIN_TEXT_CALENDAR_YEAR: i32 = 1000;
const MIN_PLAUSIBLE_UNIX_SECONDS_TEXT: i64 = 946_684_800;
const MAX_PLAUSIBLE_UNIX_SECONDS_TEXT: i64 = 4_102_444_800;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("Unix-Sekundenwert ist ausserhalb des gueltigen Zeitbereichs: {seconds}")]
    InvalidUnixTimestamp { seconds: i64 },
    #[error("REAL-Unix-Sekundenwert ist nicht gueltig: {seconds}")]
    InvalidRealUnixTimestamp { seconds: f64 },
    #[error("SQLite-Integer-Bool erwartet 0 oder 1, bekam {value}")]
    InvalidBoolInteger { value: i64 },
    #[error("TEXT-Wert ist kein gueltiger BIGINT: {value}")]
    InvalidBigintText { value: String },
    #[error("TEXT-Wert liegt ausserhalb des BIGINT-Bereichs: {value}")]
    BigintTextOutOfRange { value: String },
    #[error("TEXT-Wert ist kein gueltiger Zeitstempel: {value}")]
    InvalidTimestampText { value: String },
    #[error("TEXT-Wert ist kein gueltiges Datum: {value}")]
    InvalidDateText { value: String },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn unix_seconds_to_datetime(seconds: i64) -> Result<DateTime<Utc>, ConvertError> {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .ok_or(ConvertError::InvalidUnixTimestamp { seconds })
}

pub fn optional_unix_seconds_to_datetime(
    seconds: Option<i64>,
) -> Result<Option<DateTime<Utc>>, ConvertError> {
    seconds.map(unix_seconds_to_datetime).transpose()
}

pub fn real_unix_seconds_to_datetime(seconds: f64) -> Result<DateTime<Utc>, ConvertError> {
    if !seconds.is_finite() {
        return Err(ConvertError::InvalidRealUnixTimestamp { seconds });
    }

    let whole_seconds = seconds.floor();
    if whole_seconds < i64::MIN as f64 || whole_seconds > i64::MAX as f64 {
        return Err(ConvertError::InvalidRealUnixTimestamp { seconds });
    }

    let mut whole_seconds = whole_seconds as i64;
    let mut nanos = ((seconds - whole_seconds as f64) * 1_000_000_000_f64).round() as u32;
    if nanos == 1_000_000_000 {
        whole_seconds = whole_seconds
            .checked_add(1)
            .ok_or(ConvertError::InvalidRealUnixTimestamp { seconds })?;
        nanos = 0;
    }

    DateTime::<Utc>::from_timestamp(whole_seconds, nanos)
        .ok_or(ConvertError::InvalidRealUnixTimestamp { seconds })
}

pub fn optional_real_unix_seconds_to_datetime(
    seconds: Option<f64>,
) -> Result<Option<DateTime<Utc>>, ConvertError> {
    seconds.map(real_unix_seconds_to_datetime).transpose()
}

pub fn sqlite_int_to_bool(value: i64) -> Result<bool, ConvertError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(ConvertError::InvalidBoolInteger { value: other }),
    }
}

pub fn optional_sqlite_int_to_bool(value: Option<i64>) -> Result<Option<bool>, ConvertError> {
    value.map(sqlite_int_to_bool).transpose()
}

pub fn sqlite_numeric_to_bool(value: i64) -> Result<bool, ConvertError> {
    sqlite_int_to_bool(value)
}

pub fn optional_sqlite_numeric_to_bool(value: Option<i64>) -> Result<Option<bool>, ConvertError> {
    value.map(sqlite_numeric_to_bool).transpose()
}

pub fn text_json_to_value(text: &str) -> Result<Value, ConvertError> {
    serde_json::from_str(text).map_err(ConvertError::from)
}

pub fn text_json_or_integer_csv_list_to_value(text: &str) -> Result<Value, ConvertError> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Number(number)) if is_unsigned_integer_text(text.trim()) => {
            Ok(Value::Array(vec![Value::Number(number)]))
        }
        Ok(value) => Ok(value),
        Err(json_error) => parse_unsigned_integer_csv_list(text)
            .map(Value::Array)
            .ok_or(ConvertError::Json(json_error)),
    }
}

pub fn optional_text_json_to_value(text: Option<&str>) -> Result<Option<Value>, ConvertError> {
    text.map(text_json_to_value).transpose()
}

pub fn text_to_bigint(text: &str) -> Result<i64, ConvertError> {
    if !is_integer_text(text) {
        return Err(ConvertError::InvalidBigintText {
            value: text.to_string(),
        });
    }

    text.parse::<i64>()
        .map_err(|_| ConvertError::BigintTextOutOfRange {
            value: text.to_string(),
        })
}

pub fn optional_text_to_bigint(text: Option<&str>) -> Result<Option<i64>, ConvertError> {
    text.map(text_to_bigint).transpose()
}

pub fn integer_to_text(value: i64) -> String {
    value.to_string()
}

pub fn optional_integer_to_text(value: Option<i64>) -> Option<String> {
    value.map(integer_to_text)
}

pub fn text_to_date(text: &str) -> Result<NaiveDate, ConvertError> {
    if has_invalid_calendar_year_prefix(text) {
        return Err(ConvertError::InvalidDateText {
            value: text.to_string(),
        });
    }

    if let Ok(value) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Ok(value);
    }

    if let Ok(value) = text_to_datetime(text) {
        return Ok(value.date_naive());
    }

    Err(ConvertError::InvalidDateText {
        value: text.to_string(),
    })
}

pub fn optional_text_to_date(text: Option<&str>) -> Result<Option<NaiveDate>, ConvertError> {
    text.map(text_to_date).transpose()
}

pub fn text_to_datetime(text: &str) -> Result<DateTime<Utc>, ConvertError> {
    if has_invalid_calendar_year_prefix(text) {
        return Err(ConvertError::InvalidTimestampText {
            value: text.to_string(),
        });
    }

    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Ok(value.with_timezone(&Utc));
    }

    for format in [
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%d %H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M%z",
        "%Y-%m-%d %H:%M%z",
        "%Y-%m-%dT%H:%M%:z",
        "%Y-%m-%d %H:%M%:z",
    ] {
        if let Ok(value) = DateTime::parse_from_str(text, format) {
            return Ok(value.with_timezone(&Utc));
        }
    }

    if let Some(stripped) = text.strip_suffix('Z').or_else(|| text.strip_suffix('z')) {
        if let Ok(value) = parse_naive_datetime(stripped) {
            return Ok(value.and_utc());
        }
    }

    if let Ok(value) = parse_naive_datetime(text) {
        return Ok(value.and_utc());
    }

    if let Some(value) = parse_unix_seconds_text(text)? {
        return Ok(value);
    }

    Err(ConvertError::InvalidTimestampText {
        value: text.to_string(),
    })
}

pub fn optional_text_to_datetime(
    text: Option<&str>,
) -> Result<Option<DateTime<Utc>>, ConvertError> {
    text.map(text_to_datetime).transpose()
}

fn is_integer_text(text: &str) -> bool {
    let Some(first) = text.as_bytes().first() else {
        return false;
    };

    let digits = if *first == b'+' || *first == b'-' {
        &text.as_bytes()[1..]
    } else {
        text.as_bytes()
    };

    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn is_unsigned_integer_text(text: &str) -> bool {
    !text.is_empty() && text.as_bytes().iter().all(u8::is_ascii_digit)
}

fn parse_unsigned_integer_csv_list(text: &str) -> Option<Vec<Value>> {
    let text = text.trim();
    if text.is_empty() || text.starts_with('[') || text.ends_with(']') {
        return None;
    }

    let mut values = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if !is_unsigned_integer_text(part) {
            return None;
        }
        let value = part.parse::<u64>().ok()?;
        values.push(Value::Number(Number::from(value)));
    }

    (!values.is_empty()).then_some(values)
}

fn parse_unix_seconds_text(text: &str) -> Result<Option<DateTime<Utc>>, ConvertError> {
    if text.len() != 10 || !text.as_bytes().iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    if is_compact_calendar_datetime_text(text) {
        return Err(ConvertError::InvalidTimestampText {
            value: text.to_string(),
        });
    }

    let seconds = text
        .parse::<i64>()
        .map_err(|_| ConvertError::InvalidTimestampText {
            value: text.to_string(),
        })?;
    if !(MIN_PLAUSIBLE_UNIX_SECONDS_TEXT..=MAX_PLAUSIBLE_UNIX_SECONDS_TEXT).contains(&seconds) {
        return Err(ConvertError::InvalidTimestampText {
            value: text.to_string(),
        });
    }

    unix_seconds_to_datetime(seconds).map(Some)
}

fn is_compact_calendar_datetime_text(text: &str) -> bool {
    let Ok(year) = text[0..4].parse::<i32>() else {
        return false;
    };
    if year < MIN_TEXT_CALENDAR_YEAR {
        return false;
    }
    let Ok(month) = text[4..6].parse::<u32>() else {
        return false;
    };
    let Ok(day) = text[6..8].parse::<u32>() else {
        return false;
    };
    let Ok(hour) = text[8..10].parse::<u32>() else {
        return false;
    };

    hour <= 23 && NaiveDate::from_ymd_opt(year, month, day).is_some()
}

fn has_invalid_calendar_year_prefix(text: &str) -> bool {
    let Some((year, rest)) = text.split_once('-') else {
        return false;
    };
    if !starts_like_calendar_month_day(rest) {
        return false;
    }
    if year.len() != 4 || !year.as_bytes().iter().all(u8::is_ascii_digit) {
        return true;
    }

    year.parse::<i32>()
        .map_or(true, |year| year < MIN_TEXT_CALENDAR_YEAR)
}

fn starts_like_calendar_month_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 5
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'-'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
}

fn parse_naive_datetime(text: &str) -> Result<NaiveDateTime, chrono::ParseError> {
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(value);
        }
    }

    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
}
