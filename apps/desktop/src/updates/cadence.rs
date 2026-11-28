//! When the app looks for an update by itself: at launch, then once the last look is a day
//! old; after a look that didn't get through, again an hour later, so an offline computer or a
//! GitHub outage costs one request an hour, not one a minute. A minute's tick asks, so a
//! computer that wakes after a longer sleep looks within a minute (the checks compare
//! wall-clock times, which a sleep doesn't stop). With automatic checks off, only Check for
//! updates looks.

use std::time::{Duration, SystemTime};

/// How old the last check may get.
pub(crate) const EVERY: Duration = Duration::from_hours(24);

/// How long after a check that didn't get through the next one waits.
pub(crate) const RETRY: Duration = Duration::from_hours(1);

/// How often the app asks whether a check is due, and whether to install now.
pub(crate) const TICK: Duration = Duration::from_mins(1);

/// Whether a check is due at `now`, the last one that got through having been at `last` and the
/// last one tried at `tried` (none yet: due). A time in the future (the clock was set back)
/// doesn't hold a check off.
#[must_use]
pub(crate) fn is_due(last: Option<SystemTime>, tried: Option<SystemTime>, now: SystemTime) -> bool {
    let older_than = |at: Option<SystemTime>, wait: Duration| {
        at.is_none_or(|at| now.duration_since(at).map_or(true, |age| age >= wait))
    };
    older_than(last, EVERY) && older_than(tried, RETRY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_hours(1);

    #[test]
    fn a_check_is_due_first_and_then_once_a_day() {
        let now = SystemTime::UNIX_EPOCH + 1000 * HOUR;
        assert!(is_due(None, None, now));
        assert!(!is_due(Some(now - HOUR), Some(now - HOUR), now));
        assert!(!is_due(Some(now - 23 * HOUR), Some(now - 23 * HOUR), now));
        assert!(is_due(Some(now - EVERY), Some(now - EVERY), now));
        assert!(
            is_due(Some(now - 30 * HOUR), Some(now - 30 * HOUR), now),
            "after a long sleep"
        );
    }

    #[test]
    fn a_check_that_didnt_get_through_is_tried_again_an_hour_later() {
        let now = SystemTime::UNIX_EPOCH + 1000 * HOUR;
        let last = Some(now - 48 * HOUR);
        assert!(!is_due(last, Some(now - 10 * TICK), now));
        assert!(is_due(last, Some(now - RETRY), now));
        assert!(!is_due(None, Some(now - TICK), now), "never through yet");
    }

    #[test]
    fn a_time_in_the_future_doesnt_hold_a_check_off() {
        let now = SystemTime::UNIX_EPOCH + 1000 * HOUR;
        assert!(is_due(Some(now + HOUR), Some(now + HOUR), now));
    }
}
