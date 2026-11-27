//! What time it is, behind a trait so tests can move time on.
//!
//! Every rule with a deadline (pairing codes, access tokens, the wrong-code window) asks a
//! [`Clock`] instead of the system, so a test can step past a deadline without sleeping.

use std::fmt;
use std::sync::{Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};

/// A source of the current time.
pub trait Clock: fmt::Debug + Send + Sync {
    /// The current instant.
    fn now(&self) -> Timestamp;
}

/// The system's clock: what the server uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock that stands still until it is moved: for tests.
///
/// ```
/// use jiff::{SignedDuration, Timestamp};
/// use emberhut_core::{Clock, ManualClock};
///
/// let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
/// clock.advance(SignedDuration::from_mins(10));
/// assert_eq!(clock.now(), Timestamp::UNIX_EPOCH + SignedDuration::from_mins(10));
/// ```
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<Timestamp>,
}

impl ManualClock {
    /// A clock that reads `start` until it is moved.
    #[must_use]
    pub fn new(start: Timestamp) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Moves the clock on by `by` (or back, for a negative duration).
    ///
    /// # Panics
    ///
    /// When the result is outside the range a [`Timestamp`] can hold, which no test needs.
    pub fn advance(&self, by: SignedDuration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now += by;
    }

    /// Sets the clock to `to`.
    pub fn set(&self, to: Timestamp) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = to;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let start = Timestamp::from_second(1_000).unwrap();
        let clock = ManualClock::new(start);
        assert_eq!(clock.now(), start);
        clock.advance(SignedDuration::from_secs(5));
        assert_eq!(clock.now(), start + SignedDuration::from_secs(5));
        clock.set(start);
        assert_eq!(clock.now(), start);
    }

    #[test]
    fn system_clock_is_after_2025() {
        let jan_2025 = Timestamp::from_second(1_735_689_600).unwrap();
        assert!(SystemClock.now() > jan_2025);
    }
}
