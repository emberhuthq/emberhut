//! The directory's wall clock: what statements' times and tokens' lifetimes are measured
//! against. A trait, so tests can set the time a statement or token was made for.

use std::fmt;
use std::time::SystemTime;

use emberhut_identity::UnixMillis;

/// Tells the time.
pub trait Clock: Send + Sync + fmt::Debug + 'static {
    /// Now, in milliseconds since the Unix epoch.
    fn now(&self) -> UnixMillis;
}

/// The system's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        // Out of range only before 1970 or after the year 287,396: a broken clock, under which
        // every statement is then stale and every token expired, rather than fresh.
        UnixMillis::try_from(SystemTime::now()).unwrap_or(UnixMillis::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_is_after_the_vectors_were_made() {
        assert!(SystemClock.now().get() > 1_795_000_000_000);
    }
}
