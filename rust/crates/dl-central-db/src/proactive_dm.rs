use chrono::{DateTime, Duration, Utc};

const MINIMUM_PING_INTERVAL: Duration = Duration::days(14);
const MAX_PINGS_PER_30_DAYS: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProactiveDmEligibility {
    pub user_id: i64,
    pub opted_out: bool,
    pub last_pinged_at: Option<DateTime<Utc>>,
    pub ping_count_30d: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactiveDmDenialReason {
    OptedOut,
    CooldownActive { next_allowed_at: DateTime<Utc> },
    MonthlyBudgetExhausted,
}

pub fn is_proactive_dm_allowed(
    eligibility: ProactiveDmEligibility,
    now: DateTime<Utc>,
) -> Result<(), ProactiveDmDenialReason> {
    if eligibility.opted_out {
        return Err(ProactiveDmDenialReason::OptedOut);
    }

    if let Some(last_pinged_at) = eligibility.last_pinged_at {
        let next_allowed_at = last_pinged_at + MINIMUM_PING_INTERVAL;
        if next_allowed_at > now {
            return Err(ProactiveDmDenialReason::CooldownActive { next_allowed_at });
        }
    }

    if eligibility.ping_count_30d >= MAX_PINGS_PER_30_DAYS {
        return Err(ProactiveDmDenialReason::MonthlyBudgetExhausted);
    }

    Ok(())
}
