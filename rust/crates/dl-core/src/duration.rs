use std::time::Duration;

pub fn positive_secs(value: f64) -> Option<Duration> {
    if value > 0.0 {
        Duration::try_from_secs_f64(value).ok()
    } else {
        None
    }
}

pub fn retry_after(seconds: f64, max: Duration) -> Duration {
    if seconds.is_nan() || seconds <= 0.0 {
        return Duration::ZERO;
    }
    Duration::try_from_secs_f64(seconds).map_or(max, |wait| wait.min(max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_secs_rejects_every_unusable_value() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert_eq!(positive_secs(value), None, "{value}");
        }
        assert_eq!(positive_secs(1.5), Some(Duration::from_millis(1500)));
    }

    #[test]
    fn retry_after_never_panics_and_stays_inside_the_maximum() {
        let max = Duration::from_secs(7);
        assert_eq!(retry_after(f64::NAN, max), Duration::ZERO);
        assert_eq!(retry_after(-3.0, max), Duration::ZERO);
        assert_eq!(retry_after(f64::NEG_INFINITY, max), Duration::ZERO);
        assert_eq!(retry_after(f64::INFINITY, max), max);
        assert_eq!(retry_after(1e300, max), max);
        assert_eq!(retry_after(1e20, Duration::MAX), Duration::MAX);
        assert_eq!(retry_after(60.0, max), max);
        assert_eq!(retry_after(0.25, max), Duration::from_millis(250));
    }
}
