use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::backend::{BackendError, FailureKind};
use crate::upstream::{refusal_status, BILLING};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub provider: String,
    pub kind: &'static str,
    pub reason: &'static str,
    pub status: Option<u16>,
}

pub trait RefusalNotifier: Send + Sync {
    fn notify(&self, refusal: Refusal, delivery: Delivery);
}

#[derive(Default)]
struct Fault {
    streak: u32,
    in_flight: bool,
    reported: bool,
}

type Faults = Arc<Mutex<BTreeMap<String, Fault>>>;

fn lock(faults: &Faults) -> MutexGuard<'_, BTreeMap<String, Fault>> {
    faults.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Delivery {
    faults: Faults,
    provider: String,
}

impl Delivery {
    pub fn confirm(self) {
        if let Some(fault) = lock(&self.faults).get_mut(&self.provider) {
            fault.reported = true;
        }
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        let mut faults = lock(&self.faults);
        let idle = faults.get_mut(&self.provider).is_some_and(|fault| {
            fault.in_flight = false;
            fault.streak == 0 && !fault.reported
        });
        if idle {
            faults.remove(&self.provider);
        }
    }
}

pub struct RefusalAlarm {
    notifier: Arc<dyn RefusalNotifier>,
    streak: NonZeroU32,
    faults: Faults,
}

impl RefusalAlarm {
    pub fn new(notifier: Arc<dyn RefusalNotifier>, streak: NonZeroU32) -> Self {
        Self {
            notifier,
            streak,
            faults: Arc::default(),
        }
    }

    pub fn record(&self, provider: &str, outcome: Result<(), &BackendError>) {
        let error = match outcome {
            Ok(()) => return self.clear(provider),
            Err(error) if is_refusal(error) => error,
            Err(_) => return,
        };
        let threshold = if error.reason == BILLING {
            1
        } else {
            self.streak.get()
        };
        let mut faults = lock(&self.faults);
        let fault = faults.entry(provider.to_owned()).or_default();
        fault.streak = fault.streak.saturating_add(1);
        if fault.streak < threshold || fault.in_flight || fault.reported {
            return;
        }
        fault.in_flight = true;
        drop(faults);
        self.notifier.notify(
            Refusal {
                provider: provider.to_owned(),
                kind: refusal_kind(error.kind),
                reason: error.reason,
                status: error.status,
            },
            Delivery {
                faults: Arc::clone(&self.faults),
                provider: provider.to_owned(),
            },
        );
    }

    fn clear(&self, provider: &str) {
        let mut faults = lock(&self.faults);
        let in_flight = faults.get_mut(provider).is_some_and(|fault| {
            fault.streak = 0;
            fault.reported = false;
            fault.in_flight
        });
        if !in_flight {
            faults.remove(provider);
        }
    }
}

fn is_refusal(error: &BackendError) -> bool {
    matches!(error.kind, FailureKind::Limit | FailureKind::Overloaded)
        || error.status.is_some_and(refusal_status)
}

fn refusal_kind(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Limit => "limit",
        FailureKind::Overloaded => "overloaded",
        _ => "unavailable",
    }
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
        fn sent(&self) -> Vec<Refusal> {
            self.sent
                .lock()
                .map(|sent| sent.clone())
                .unwrap_or_default()
        }

        fn finish(&self, confirmed: bool) {
            let held: Vec<Delivery> = self
                .held
                .lock()
                .map(|mut held| held.drain(..).collect())
                .unwrap_or_default();
            for delivery in held {
                if confirmed {
                    delivery.confirm();
                }
            }
        }
    }

    fn alarm(streak: u32) -> (Arc<Recorder>, RefusalAlarm) {
        let recorder = Arc::new(Recorder::default());
        let streak = NonZeroU32::new(streak).unwrap_or(NonZeroU32::MIN);
        (recorder.clone(), RefusalAlarm::new(recorder, streak))
    }

    #[test]
    fn billing_alarms_on_the_first_refusal_and_once_per_standing_fault() {
        let (recorder, alarm) = alarm(5);
        let billing = BackendError::upstream(FailureKind::Limit, BILLING, 400);
        for _ in 0..288 {
            alarm.record("anthropic_api", Err(&billing));
        }
        assert_eq!(
            recorder.sent(),
            vec![Refusal {
                provider: "anthropic_api".into(),
                kind: "limit",
                reason: BILLING,
                status: Some(400),
            }]
        );
        recorder.finish(false);
        alarm.record("anthropic_api", Err(&billing));
        assert_eq!(recorder.sent().len(), 2);
        recorder.finish(true);
        for _ in 0..10 {
            alarm.record("anthropic_api", Err(&billing));
        }
        assert_eq!(recorder.sent().len(), 2);
        alarm.record("anthropic_api", Ok(()));
        alarm.record("anthropic_api", Err(&billing));
        assert_eq!(recorder.sent().len(), 3);
    }

    #[test]
    fn other_refusals_alarm_on_a_mixed_streak_that_success_resets() {
        let (recorder, alarm) = alarm(3);
        let limited = BackendError::upstream(FailureKind::Limit, "http_429", 429);
        let overloaded = BackendError::upstream(FailureKind::Overloaded, "http_529", 529);
        alarm.record("anthropic_api", Err(&limited));
        alarm.record("anthropic_api", Err(&overloaded));
        alarm.record("anthropic_api", Ok(()));
        alarm.record("anthropic_api", Err(&limited));
        alarm.record("claude_plan", Err(&limited));
        alarm.record("anthropic_api", Err(&overloaded));
        assert!(recorder.sent().is_empty());
        alarm.record("anthropic_api", Err(&limited));
        assert_eq!(
            recorder.sent(),
            vec![Refusal {
                provider: "anthropic_api".into(),
                kind: "limit",
                reason: "http_429",
                status: Some(429),
            }]
        );
    }

    #[test]
    fn local_and_caller_failures_neither_count_nor_reset() {
        let (recorder, alarm) = alarm(2);
        let limited = BackendError::upstream(FailureKind::Limit, "http_429", 429);
        alarm.record("anthropic_api", Err(&limited));
        for error in [
            BackendError::new(FailureKind::Busy, "queue_timeout"),
            BackendError::new(FailureKind::Timeout, "slot_timeout"),
            BackendError::new(FailureKind::Unavailable, "connect"),
            BackendError::upstream(FailureKind::InvalidRequest, "invalid_request_error", 400),
            BackendError::upstream(FailureKind::Unavailable, "http_4xx", 400),
        ] {
            alarm.record("anthropic_api", Err(&error));
        }
        assert!(recorder.sent().is_empty());
        alarm.record(
            "anthropic_api",
            Err(&BackendError::upstream(
                FailureKind::Unavailable,
                "authentication_error",
                401,
            )),
        );
        assert_eq!(recorder.sent().len(), 1);
        assert_eq!(recorder.sent()[0].kind, "unavailable");
    }
}
