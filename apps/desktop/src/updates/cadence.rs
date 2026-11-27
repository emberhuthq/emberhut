//! When the app looks for an update: at launch, then once the last look is 4 hours old. A
//! minute's tick asks, so a Mac that wakes after a longer sleep looks within a minute (the
//! check compares wall-clock times, which a sleep doesn't stop).

use std::time::{Duration, SystemTime};

/// How old the last check may get.
pub(crate) const EVERY: Duration = Duration::from_hours(4);

/// How often the app asks whether a check is due, and whether to install now.
pub(crate) const TICK: Duration = Duration::from_mins(1);

/// Whether a check is due at `now`, the last one having been at `last` (none yet: due). A last
/// check in the future (the clock was set back) is due too, rather than waiting for the clock.
#[must_use]
pub(crate) fn is_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    let Some(last) = last else {
        return true;
    };
    match now.duration_since(last) {
        Ok(age) => age >= EVERY,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_hours(1);

    #[test]
    fn a_check_is_due_first_and_then_every_four_hours() {
        let now = SystemTime::UNIX_EPOCH + 1000 * HOUR;
        assert!(is_due(None, now));
        assert!(!is_due(Some(now - HOUR), now));
        assert!(!is_due(Some(now - EVERY + Duration::from_secs(1)), now));
        assert!(is_due(Some(now - EVERY), now));
        assert!(is_due(Some(now - 30 * HOUR), now), "after a long sleep");
    }

    #[test]
    fn a_last_check_in_the_future_is_due() {
        let now = SystemTime::UNIX_EPOCH + 1000 * HOUR;
        assert!(is_due(Some(now + HOUR), now));
    }
}
