use std::num::TryFromIntError;

#[derive(Debug, thiserror::Error)]
pub enum TierlistError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("Unix-Zeitstempel ausserhalb des gueltigen Bereichs: {0}")]
    TimestampOutOfRange(i64),
    #[error("Wert {field}={value} passt nicht in PostgreSQL int4")]
    IntOutOfRange {
        field: &'static str,
        value: i64,
        source: TryFromIntError,
    },
}

pub type Result<T> = std::result::Result<T, TierlistError>;
