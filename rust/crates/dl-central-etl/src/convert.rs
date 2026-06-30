use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("Unix-Sekundenwert ist ausserhalb des gueltigen Zeitbereichs: {seconds}")]
    InvalidUnixTimestamp { seconds: i64 },
    #[error("SQLite-Integer-Bool erwartet 0 oder 1, bekam {value}")]
    InvalidBoolInteger { value: i64 },
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

pub fn text_json_to_value(text: &str) -> Result<Value, ConvertError> {
    serde_json::from_str(text).map_err(ConvertError::from)
}

pub fn optional_text_json_to_value(text: Option<&str>) -> Result<Option<Value>, ConvertError> {
    text.map(text_json_to_value).transpose()
}
