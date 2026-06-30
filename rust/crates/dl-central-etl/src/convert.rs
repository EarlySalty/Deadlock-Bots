use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;

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
    if let Ok(value) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Ok(value);
    }

    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Ok(value.date_naive());
    }

    for format in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(value.date());
        }
    }

    Err(ConvertError::InvalidDateText {
        value: text.to_string(),
    })
}

pub fn optional_text_to_date(text: Option<&str>) -> Result<Option<NaiveDate>, ConvertError> {
    text.map(text_to_date).transpose()
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
