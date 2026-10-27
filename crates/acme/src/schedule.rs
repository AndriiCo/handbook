//! Renewal scheduling: the CA's ARI window first (RFC 9773 §4.2),
//! a certificate-lifetime fallback when ARI is unavailable, and
//! backoff after failures. All pure functions over [`SystemTime`].

use std::time::{Duration, SystemTime};

/// Renew once remaining lifetime drops to this (fallback path only;
/// the ARI window decides when the CA offers one).
pub const FALLBACK_RENEW_BEFORE: Duration = Duration::from_secs(30 * 24 * 3600);
/// Re-check cadence while the fallback path waits for the window.
pub const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// One ARI sleep never exceeds this; the loop re-fetches the window
/// after waking (the CA moves windows on revocation events).
pub const MAX_ARI_SLEEP: Duration = Duration::from_secs(24 * 3600);
/// Delay after the first failed issuance; doubles per consecutive failure.
pub const RETRY_BASE: Duration = Duration::from_secs(5 * 60);
/// Failure backoff never exceeds this.
pub const RETRY_MAX: Duration = FALLBACK_POLL_INTERVAL;

/// How long to sleep before attempting renewal under an ARI window.
///
/// `jitter01` in `[0, 1]` picks the renewal point uniformly inside
/// `[start, end]` per RFC 9773 §4.2 (callers pass `rand::random()`;
/// out-of-range values clamp). `Duration::ZERO` means renew now:
/// the window already started or fully passed. A `Retry-After` hint
/// shortens the sleep so the client re-fetches sooner.
pub fn ari_sleep(
    now: SystemTime,
    start: SystemTime,
    end: SystemTime,
    retry_after: Option<Duration>,
    jitter01: f64,
) -> Duration {
    if now >= end || now >= start {
        return Duration::ZERO;
    }
    let span = end.duration_since(start).unwrap_or(Duration::ZERO);
    let point = start + span.mul_f64(jitter01.clamp(0.0, 1.0));
    let mut sleep = point.duration_since(now).unwrap_or(Duration::ZERO);
    if let Some(hint) = retry_after {
        sleep = sleep.min(hint);
    }
    sleep.min(MAX_ARI_SLEEP)
}

/// True when the fallback path must renew now: expired, or at most
/// [`FALLBACK_RENEW_BEFORE`] of lifetime left.
pub fn fallback_due(not_after: SystemTime, now: SystemTime) -> bool {
    match not_after.duration_since(now) {
        Ok(remaining) => remaining <= FALLBACK_RENEW_BEFORE,
        Err(_) => true,
    }
}

/// How long the fallback path sleeps: zero when
/// [`fallback_due`], otherwise the sooner of the poll interval and
/// the moment the renewal window opens.
pub fn fallback_sleep(not_after: SystemTime, now: SystemTime) -> Duration {
    if fallback_due(not_after, now) {
        return Duration::ZERO;
    }
    let until_due = not_after
        .duration_since(now)
        .unwrap_or(Duration::ZERO)
        .saturating_sub(FALLBACK_RENEW_BEFORE);
    until_due.min(FALLBACK_POLL_INTERVAL)
}

/// Backoff after `consecutive_failures` failed attempts (failures=0
/// is the first retry): 5 min doubling to a 6 h cap.
pub fn retry_sleep(consecutive_failures: u32) -> Duration {
    let mut delay = RETRY_BASE;
    for _ in 0..consecutive_failures.min(8) {
        delay = delay.saturating_mul(2);
        if delay >= RETRY_MAX {
            break;
        }
    }
    delay.min(RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);
    const DAY: Duration = Duration::from_secs(24 * 3600);

    fn t(offset: Duration) -> SystemTime {
        SystemTime::UNIX_EPOCH + offset
    }

    #[test]
    fn ari_window_edges_renew_now() {
        let now = t(100 * DAY);
        // Inside the window, at its start, or past its end: now.
        assert!(ari_sleep(now, t(90 * DAY), t(110 * DAY), None, 0.5).is_zero());
        assert!(ari_sleep(now, now, t(110 * DAY), None, 0.5).is_zero());
        assert!(ari_sleep(now, t(80 * DAY), t(90 * DAY), None, 0.5).is_zero());
        assert!(ari_sleep(now, t(80 * DAY), now, None, 0.5).is_zero());
    }

    #[test]
    fn ari_future_window_sleeps_into_it() {
        let now = t(100 * DAY);
        // Far window: any jittered point exceeds the 24h cap.
        assert_eq!(
            ari_sleep(now, t(110 * DAY), t(120 * DAY), None, 0.0),
            MAX_ARI_SLEEP
        );
        // Near window: jitter 0.0 lands exactly on the start.
        assert_eq!(ari_sleep(now, now + HOUR, now + 3 * HOUR, None, 0.0), HOUR);
        // Near window: jitter 0.5 lands exactly in the middle.
        assert_eq!(
            ari_sleep(now, now + HOUR, now + 3 * HOUR, None, 0.5),
            2 * HOUR
        );
    }

    #[test]
    fn ari_retry_after_shortens_and_clamps_hard() {
        let now = t(100 * DAY);
        let sleep = ari_sleep(now, t(110 * DAY), t(120 * DAY), Some(HOUR), 1.0);
        assert_eq!(sleep, HOUR);
        // A huge hint still cannot exceed the cap.
        let sleep = ari_sleep(now, t(110 * DAY), t(120 * DAY), Some(30 * DAY), 1.0);
        assert_eq!(sleep, MAX_ARI_SLEEP);
    }

    #[test]
    fn fallback_due_and_sleep() {
        let now = t(100 * DAY);
        assert!(fallback_due(now - HOUR, now), "expired is due");
        assert!(fallback_due(now + 29 * DAY, now), "29d left is due");
        assert!(!fallback_due(now + 31 * DAY, now), "31d left waits");
        assert!(fallback_sleep(now + 10 * DAY, now).is_zero());
        // 31d left: due in 1d, but the poll interval binds first.
        assert_eq!(fallback_sleep(now + 31 * DAY, now), FALLBACK_POLL_INTERVAL);
        // 30d + 1h left: due in exactly 1h.
        assert_eq!(fallback_sleep(now + 30 * DAY + HOUR, now), HOUR);
    }

    #[test]
    fn retry_backoff_doubles_to_cap() {
        assert_eq!(retry_sleep(0), Duration::from_secs(300));
        assert_eq!(retry_sleep(1), Duration::from_secs(600));
        assert_eq!(retry_sleep(2), Duration::from_secs(1200));
        assert_eq!(retry_sleep(100), RETRY_MAX);
        assert_eq!(retry_sleep(u32::MAX), RETRY_MAX);
    }
}
