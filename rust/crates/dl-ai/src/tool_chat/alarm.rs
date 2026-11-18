use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::backend::FailureKind;
use crate::upstream::{
    is_account_refusal, refusal_status, safe_code, status_reason, Answer, Class, RefusalKind,
    FAILED_RUN_PAUSE_SECS, WEEK_SECS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerFailure {
    pub provider: String,
    pub error_code: Option<String>,
    pub error_reason: Option<String>,
    pub http_status: Option<i32>,
    pub first_at: i64,
    pub last_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub provider: String,
    pub class: Class,
    pub kind: RefusalKind,
    pub reason: String,
    pub status: Option<u16>,
    pub first_at: i64,
    pub last_at: i64,
}

impl Refusal {
    pub fn event_id(&self) -> String {
        format!("{}-{}", self.first_at, self.last_at)
    }
}

pub trait RefusalNotifier: Send + Sync {
    fn notify(&self, refusal: Refusal, delivery: Delivery);
}

type Key = (String, Class);

#[derive(Default)]
struct Slot {
    in_flight: bool,
    not_before: i64,
}

type Slots = Arc<Mutex<HashMap<Key, Slot>>>;

fn lock(slots: &Slots) -> MutexGuard<'_, HashMap<Key, Slot>> {
    slots.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Delivery {
    slots: Slots,
    key: Key,
    started: i64,
    answered: bool,
}

impl Delivery {
    pub fn answered(mut self, answer: Answer) {
        let latest = self.started.saturating_add(WEEK_SECS as i64);
        let next = i64::try_from(answer.next_at).map_or(latest, |next| next.min(latest));
        self.release(next);
        self.answered = true;
    }

    fn release(&self, not_before: i64) {
        if let Some(slot) = lock(&self.slots).get_mut(&self.key) {
            slot.in_flight = false;
            slot.not_before = not_before;
        }
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        if !self.answered {
            self.release(self.started.saturating_add(FAILED_RUN_PAUSE_SECS as i64));
        }
    }
}

pub struct RefusalAlarm {
    notifier: Arc<dyn RefusalNotifier>,
    standing_after: i64,
    slots: Slots,
}

impl RefusalAlarm {
    pub fn new(notifier: Arc<dyn RefusalNotifier>, standing_after_secs: u64) -> Self {
        Self {
            notifier,
            standing_after: i64::try_from(standing_after_secs).unwrap_or(i64::MAX),
            slots: Arc::default(),
        }
    }

    pub fn observe(&self, rows: &[LedgerFailure], now: i64) {
        for refusal in standing_refusals(rows, self.standing_after) {
            let key = (refusal.provider.clone(), refusal.class);
            {
                let mut slots = lock(&self.slots);
                let slot = slots.entry(key.clone()).or_default();
                if slot.in_flight || now < slot.not_before {
                    continue;
                }
                slot.in_flight = true;
            }
            self.notifier.notify(
                refusal,
                Delivery {
                    slots: Arc::clone(&self.slots),
                    key,
                    started: now,
                    answered: false,
                },
            );
        }
    }
}

pub fn standing_refusals(rows: &[LedgerFailure], standing_after: i64) -> Vec<Refusal> {
    let mut faults: HashMap<Key, Refusal> = HashMap::new();
    for refusal in rows.iter().filter_map(refusal) {
        faults
            .entry((refusal.provider.clone(), refusal.class))
            .and_modify(|known| {
                let first_at = known.first_at.min(refusal.first_at);
                if refusal.last_at > known.last_at {
                    *known = refusal.clone();
                }
                known.first_at = first_at;
            })
            .or_insert(refusal);
    }
    let mut standing: Vec<Refusal> = faults
        .into_values()
        .filter(|fault| {
            fault.class == Class::Billing
                || fault.last_at.saturating_sub(fault.first_at) >= standing_after
        })
        .collect();
    standing.sort_by(|left, right| {
        (&left.provider, left.class.as_str()).cmp(&(&right.provider, right.class.as_str()))
    });
    standing
}

fn refusal(row: &LedgerFailure) -> Option<Refusal> {
    if !safe_code(&row.provider) {
        tracing::warn!(
            provider_len = row.provider.len(),
            "AI_GATEWAY_ALARM_INVALID_PROVIDER"
        );
        return None;
    }
    let status = row
        .http_status
        .and_then(|status| u16::try_from(status).ok());
    let code = row.error_code.as_deref();
    let stored = row
        .error_reason
        .as_deref()
        .filter(|reason| safe_code(reason));
    let account = stored.is_some_and(is_account_refusal);
    let limited = code == Some(FailureKind::Limit.ledger_code());
    let overloaded = code == Some(FailureKind::Overloaded.ledger_code());
    if !account && !limited && !overloaded && !status.is_some_and(refusal_status) {
        return None;
    }
    let kind = match status {
        _ if account || limited => RefusalKind::Limit,
        Some(429) => RefusalKind::Limit,
        _ if overloaded => RefusalKind::Overloaded,
        Some(529) => RefusalKind::Overloaded,
        _ => RefusalKind::Unavailable,
    };
    let reason = stored
        .or_else(|| status.map(status_reason))
        .or(code.filter(|code| safe_code(code)))?
        .to_owned();
    Some(Refusal {
        provider: row.provider.clone(),
        class: Class::of(&reason),
        kind,
        reason,
        status,
        first_at: row.first_at,
        last_at: row.last_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        sent: Mutex<Vec<Refusal>>,
        held: Mutex<Vec<Delivery>>,
    }

    impl RefusalNotifier for Recorder {
        fn notify(&self, refusal: Refusal, delivery: Delivery) {
            if let Ok(mut sent) = self.sent.lock() {
                sent.push(refusal);
            }
            if let Ok(mut held) = self.held.lock() {
                held.push(delivery);
            }
        }
    }

    impl Recorder {
        fn count(&self) -> usize {
            self.sent.lock().map(|sent| sent.len()).unwrap_or_default()
        }

        fn finish(&self, answer: Option<Answer>) {
            let held: Vec<Delivery> = self
                .held
                .lock()
                .map(|mut held| held.drain(..).collect())
                .unwrap_or_default();
            for delivery in held {
                if let Some(answer) = answer {
                    delivery.answered(answer);
                }
            }
        }
    }

    fn row(code: &str, reason: Option<&str>, status: Option<i32>, at: (i64, i64)) -> LedgerFailure {
        LedgerFailure {
            provider: "anthropic_api".into(),
            error_code: Some(code.into()),
            error_reason: reason.map(str::to_owned),
            http_status: status,
            first_at: at.0,
            last_at: at.1,
        }
    }

    #[test]
    fn billing_fires_on_first_row_and_other_refusals_only_when_standing() {
        let billing = row("rate_limited", Some("billing"), Some(400), (100, 100));
        let limited = row("rate_limited", Some("http_429"), Some(429), (100, 699));
        let fired = standing_refusals(&[billing.clone(), limited.clone()], 600);
        assert_eq!(fired.len(), 1);
        assert_eq!(
            (fired[0].class, fired[0].kind, fired[0].reason.as_str()),
            (Class::Billing, RefusalKind::Limit, "billing")
        );
        let overloaded = row("overloaded", None, Some(529), (650, 700));
        let fired = standing_refusals(&[limited, overloaded], 600);
        assert_eq!(fired.len(), 1);
        assert_eq!(
            (fired[0].class, fired[0].kind, fired[0].reason.as_str()),
            (Class::Other, RefusalKind::Overloaded, "http_529")
        );
        assert_eq!((fired[0].first_at, fired[0].last_at), (100, 700));
    }

    #[test]
    fn caller_and_local_failures_are_no_refusals() {
        let rows = [
            row("busy", Some("queue_timeout"), None, (0, 10_000)),
            row("timeout", Some("http_timeout"), None, (0, 10_000)),
            row("invalid_request", Some("http_4xx"), Some(400), (0, 10_000)),
            row("transport", Some("transport"), None, (0, 10_000)),
            LedgerFailure {
                provider: "Anthropic API".into(),
                ..row("rate_limited", Some("billing"), Some(402), (0, 1))
            },
        ];
        assert!(standing_refusals(&rows, 600).is_empty());
        let auth = row("http_error", None, Some(401), (0, 600));
        let fired = standing_refusals(&[auth], 600);
        assert_eq!(
            (fired[0].kind, fired[0].reason.as_str()),
            (RefusalKind::Unavailable, "http_401")
        );
    }

    #[test]
    fn account_refusals_count_by_their_reason_whatever_code_and_status_say() {
        let billed = row("http_error", Some("billing"), Some(400), (100, 100));
        let fired = standing_refusals(&[billed], 600);
        assert_eq!(fired.len(), 1);
        assert_eq!(
            (fired[0].class, fired[0].kind, fired[0].reason.as_str()),
            (Class::Billing, RefusalKind::Limit, "billing")
        );
        let quota = [row(
            "http_error",
            Some("usage_limit"),
            Some(400),
            (100, 699),
        )];
        assert!(standing_refusals(&quota, 601).is_empty());
        let fired = standing_refusals(&quota, 599);
        assert_eq!(
            (fired[0].class, fired[0].kind, fired[0].reason.as_str()),
            (Class::Other, RefusalKind::Limit, "usage_limit")
        );
        let plain = row("http_error", Some("http_4xx"), Some(400), (0, 10_000));
        let unsafe_reason = row("http_error", Some("Billing"), Some(400), (0, 10_000));
        let missing = row("http_error", None, Some(400), (0, 10_000));
        assert!(standing_refusals(&[plain, unsafe_reason, missing], 600).is_empty());
    }

    #[test]
    fn only_the_helper_answer_sets_the_pace_and_a_failed_run_pauses() {
        let recorder = Arc::new(Recorder::default());
        let alarm = RefusalAlarm::new(recorder.clone(), 600);
        let billing = [row("rate_limited", Some("billing"), Some(400), (100, 100))];
        alarm.observe(&billing, 100);
        alarm.observe(&billing, 160);
        assert_eq!(recorder.count(), 1);
        recorder.finish(None);
        alarm.observe(&billing, 100 + FAILED_RUN_PAUSE_SECS as i64 - 1);
        assert_eq!(recorder.count(), 1);
        let retry_at = 100 + FAILED_RUN_PAUSE_SECS as i64;
        alarm.observe(&billing, retry_at);
        assert_eq!(recorder.count(), 2);
        recorder.finish(Some(Answer {
            outcome: crate::upstream::Outcome::Suppressed,
            next_at: 50_000,
        }));
        alarm.observe(&billing, 49_999);
        assert_eq!(recorder.count(), 2);
        alarm.observe(&billing, 50_000);
        assert_eq!(recorder.count(), 3);
        recorder.finish(Some(Answer {
            outcome: crate::upstream::Outcome::Sent,
            next_at: u64::MAX,
        }));
        alarm.observe(&billing, 50_000 + WEEK_SECS as i64);
        assert_eq!(recorder.count(), 4);
    }
}
