use chrono::{Duration, TimeZone, Utc};
use dl_central_db::{is_proactive_dm_allowed, ProactiveDmDenialReason, ProactiveDmEligibility};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
        .single()
        .expect("valid timestamp")
}

fn eligible() -> ProactiveDmEligibility {
    ProactiveDmEligibility {
        user_id: 42,
        opted_out: false,
        last_pinged_at: None,
        ping_count_30d: 0,
    }
}

#[test]
fn allows_user_with_free_cooldown_and_budget() {
    assert_eq!(is_proactive_dm_allowed(eligible(), now()), Ok(()));
}

#[test]
fn denies_opted_out_user_first() {
    let input = ProactiveDmEligibility {
        opted_out: true,
        last_pinged_at: Some(now() - Duration::days(1)),
        ping_count_30d: 2,
        ..eligible()
    };

    assert_eq!(
        is_proactive_dm_allowed(input, now()),
        Err(ProactiveDmDenialReason::OptedOut)
    );
}

#[test]
fn denies_user_pinged_less_than_fourteen_days_ago() {
    let last_pinged_at = now() - Duration::days(13);
    let input = ProactiveDmEligibility {
        last_pinged_at: Some(last_pinged_at),
        ..eligible()
    };

    assert_eq!(
        is_proactive_dm_allowed(input, now()),
        Err(ProactiveDmDenialReason::CooldownActive {
            next_allowed_at: last_pinged_at + Duration::days(14),
        })
    );
}

#[test]
fn allows_user_exactly_fourteen_days_after_last_ping() {
    let input = ProactiveDmEligibility {
        last_pinged_at: Some(now() - Duration::days(14)),
        ..eligible()
    };

    assert_eq!(is_proactive_dm_allowed(input, now()), Ok(()));
}

#[test]
fn denies_user_at_monthly_budget_limit() {
    let input = ProactiveDmEligibility {
        last_pinged_at: Some(now() - Duration::days(20)),
        ping_count_30d: 2,
        ..eligible()
    };

    assert_eq!(
        is_proactive_dm_allowed(input, now()),
        Err(ProactiveDmDenialReason::MonthlyBudgetExhausted)
    );
}
