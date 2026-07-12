use chrono::{DateTime, Duration, Utc};

const MINIMUM_PING_INTERVAL: Duration = Duration::days(14);
const PING_WINDOW: Duration = Duration::days(30);
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

    if eligibility.ping_count_30d >= MAX_PINGS_PER_30_DAYS
        && eligibility
            .last_pinged_at
            .is_some_and(|last_pinged_at| last_pinged_at + PING_WINDOW > now)
    {
        return Err(ProactiveDmDenialReason::MonthlyBudgetExhausted);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    fn eligibility(last_pinged_at: Option<DateTime<Utc>>) -> ProactiveDmEligibility {
        ProactiveDmEligibility {
            user_id: 42,
            opted_out: false,
            last_pinged_at,
            ping_count_30d: 2,
        }
    }

    #[test]
    fn allows_exhausted_counter_after_ping_window_expires() {
        let input = eligibility(Some(now() - Duration::days(31)));

        assert_eq!(is_proactive_dm_allowed(input, now()), Ok(()));
    }

    #[test]
    fn denies_exhausted_counter_within_ping_window() {
        let input = eligibility(Some(now() - Duration::days(20)));

        assert_eq!(
            is_proactive_dm_allowed(input, now()),
            Err(ProactiveDmDenialReason::MonthlyBudgetExhausted)
        );
    }

    #[test]
    fn allows_exhausted_counter_without_ping_history() {
        let input = eligibility(None);

        assert_eq!(is_proactive_dm_allowed(input, now()), Ok(()));
    }

    #[test]
    fn prioritizes_cooldown_over_exhausted_counter() {
        let last_pinged_at = now() - Duration::days(10);
        let input = eligibility(Some(last_pinged_at));

        assert_eq!(
            is_proactive_dm_allowed(input, now()),
            Err(ProactiveDmDenialReason::CooldownActive {
                next_allowed_at: last_pinged_at + MINIMUM_PING_INTERVAL,
            })
        );
    }
}
