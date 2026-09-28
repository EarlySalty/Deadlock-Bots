use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

const GLOBAL_CAP: i64 = 100_000;
const OPTIONAL_CAP: i64 = 90_000;
const WINDOW_HOURS: i64 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerClass {
    OptionalPatch,
    Standard,
}

impl CallerClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptionalPatch => "optional_patch",
            Self::Standard => "standard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenialReason {
    Cooldown,
    GlobalCap,
    OptionalCap,
}

impl DenialReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cooldown => "cooldown",
            Self::GlobalCap => "global_cap",
            Self::OptionalCap => "optional_cap",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    Granted {
        id: i64,
        reserved_at: DateTime<Utc>,
    },
    Denied {
        reason: DenialReason,
        retry_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub response_at: DateTime<Utc>,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub duplicate: bool,
}

#[derive(sqlx::FromRow)]
struct StoredReservation {
    reserved_at: DateTime<Utc>,
    response_at: Option<DateTime<Utc>>,
    http_status: Option<i16>,
    retry_after: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("invalid caller")]
    InvalidCaller,
    #[error("reservation not found")]
    NotFound,
    #[error("reservation already reported with a different result")]
    ConflictingReport,
    #[error("invalid HTTP status or Retry-After header")]
    InvalidObservation,
}

pub fn valid_caller(caller: &str) -> bool {
    caller.len() <= 64
        && caller
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        && caller
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(byte))
}

pub async fn reserve(
    pool: &PgPool,
    caller: &str,
    class: CallerClass,
) -> Result<Reservation, LedgerError> {
    if !valid_caller(caller) {
        return Err(LedgerError::InvalidCaller);
    }

    let mut tx = pool.begin().await?;
    let cooldown_until: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT cooldown_until
           FROM steam.web_api_budget
          WHERE id = true
          FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;

    if let Some(retry_at) = cooldown_until.filter(|deadline| *deadline > now) {
        tx.commit().await?;
        return Ok(Reservation::Denied {
            reason: DenialReason::Cooldown,
            retry_at,
        });
    }

    let (total, optional, first_total, first_optional): (
        i64,
        i64,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT count(*),
                count(*) FILTER (WHERE caller_class = 'optional_patch'),
                min(reserved_at),
                min(reserved_at) FILTER (WHERE caller_class = 'optional_patch')
           FROM steam.web_api_reservations
          WHERE reserved_at > $1",
    )
    .bind(now - Duration::hours(WINDOW_HOURS))
    .fetch_one(&mut *tx)
    .await?;

    if total >= GLOBAL_CAP {
        tx.commit().await?;
        return Ok(Reservation::Denied {
            reason: DenialReason::GlobalCap,
            retry_at: first_total.expect("full window has an oldest reservation")
                + Duration::hours(WINDOW_HOURS),
        });
    }
    if class == CallerClass::OptionalPatch && optional >= OPTIONAL_CAP {
        tx.commit().await?;
        return Ok(Reservation::Denied {
            reason: DenialReason::OptionalCap,
            retry_at: first_optional.expect("full optional window has an oldest reservation")
                + Duration::hours(WINDOW_HOURS),
        });
    }

    let (id, reserved_at): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO steam.web_api_reservations (caller, caller_class, reserved_at)
         VALUES ($1, $2, $3)
         RETURNING id, reserved_at",
    )
    .bind(caller)
    .bind(class.as_str())
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Reservation::Granted { id, reserved_at })
}

pub async fn observe(
    pool: &PgPool,
    reservation_id: i64,
    http_status: Option<i16>,
    retry_after: Option<&str>,
) -> Result<Observation, LedgerError> {
    if reservation_id <= 0
        || http_status.is_some_and(|status| !(100..=599).contains(&status))
        || (retry_after.is_some() && http_status != Some(429))
    {
        return Err(LedgerError::InvalidObservation);
    }
    let retry_after = retry_after.filter(|header| header.len() <= 128);

    let mut tx = pool.begin().await?;
    let (previous_cooldown, previous_streak): (Option<DateTime<Utc>>, i32) = sqlx::query_as(
        "SELECT cooldown_until, consecutive_429
           FROM steam.web_api_budget
          WHERE id = true
          FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let previous: Option<StoredReservation> = sqlx::query_as(
        "SELECT reserved_at, response_at, http_status, retry_after
           FROM steam.web_api_reservations
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(reservation_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(previous) = previous else {
        return Err(LedgerError::NotFound);
    };
    if let Some(response_at) = previous.response_at {
        if previous.http_status != http_status || previous.retry_after.as_deref() != retry_after {
            return Err(LedgerError::ConflictingReport);
        }
        tx.commit().await?;
        return Ok(Observation {
            response_at,
            cooldown_until: previous_cooldown,
            duplicate: true,
        });
    }

    sqlx::query(
        "UPDATE steam.web_api_reservations
            SET response_at = $1, http_status = $2, retry_after = $3
          WHERE id = $4",
    )
    .bind(now)
    .bind(http_status)
    .bind(retry_after)
    .bind(reservation_id)
    .execute(&mut *tx)
    .await?;

    let cooldown_until = if http_status == Some(429) {
        let streak = previous_streak.saturating_add(1);
        let fallback = [30, 60, 120, 300, 900][usize::try_from(streak.saturating_sub(1))
            .unwrap_or(usize::MAX)
            .min(4)];
        let deadline = parse_retry_after(retry_after, now)
            .unwrap_or_else(|| now + Duration::seconds(fallback));
        let deadline = previous_cooldown.map_or(deadline, |prior| prior.max(deadline));
        sqlx::query(
            "UPDATE steam.web_api_budget
                SET cooldown_until = $1, consecutive_429 = $2
              WHERE id = true",
        )
        .bind(deadline)
        .bind(streak)
        .execute(&mut *tx)
        .await?;
        Some(deadline)
    } else {
        if previous_cooldown.is_some_and(|prior| previous.reserved_at >= prior)
            && previous_streak != 0
        {
            sqlx::query("UPDATE steam.web_api_budget SET consecutive_429 = 0 WHERE id = true")
                .execute(&mut *tx)
                .await?;
        }
        previous_cooldown
    };

    tx.commit().await?;
    Ok(Observation {
        response_at: now,
        cooldown_until,
        duplicate: false,
    })
}

fn parse_retry_after(header: Option<&str>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let header = header?.trim();
    if let Ok(seconds) = header.parse::<i64>() {
        if seconds >= 0 {
            return Duration::try_seconds(seconds.max(1))
                .and_then(|delta| now.checked_add_signed(delta))
                .or_else(|| now.checked_add_signed(Duration::days(365)));
        }
    }
    DateTime::parse_from_rfc2822(header)
        .ok()
        .map(|deadline| deadline.with_timezone(&Utc).max(now + Duration::seconds(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_and_retry_after_validation() {
        assert!(valid_caller("patchnotes"));
        assert!(valid_caller("twitch_bot-1"));
        assert!(!valid_caller(""));
        assert!(!valid_caller("../../other"));
        assert!(!valid_caller("SteamBot"));
        assert!(!valid_caller(&"x".repeat(65)));
        let now = DateTime::parse_from_rfc3339("2026-09-28T12:00:00Z")
            .expect("fixed time")
            .with_timezone(&Utc);
        assert_eq!(
            parse_retry_after(Some("120"), now),
            Some(now + Duration::seconds(120))
        );
        assert_eq!(
            parse_retry_after(Some("0"), now),
            Some(now + Duration::seconds(1))
        );
        assert_eq!(
            parse_retry_after(Some("Mon, 28 Sep 2026 12:03:00 GMT"), now),
            Some(now + Duration::seconds(180))
        );
        assert_eq!(parse_retry_after(Some("invalid"), now), None);
    }
}
