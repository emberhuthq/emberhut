//! The flaky-network test mode (`--flaky`, debug builds only): the state poll fails as if the
//! connection dropped for 6 seconds of every 20, to see what reconnects do to the chat, the
//! offline line and the status.

use std::time::SystemTime;

/// Whether the test mode is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The connection is as it is.
    #[default]
    Off,
    /// The connection drops 6 s of every 20.
    Flaky,
}

impl Mode {
    /// Whether a poll at `now` fails as if the connection dropped.
    #[must_use]
    pub(crate) fn drops_at(self, now: SystemTime) -> bool {
        self == Self::Flaky && is_down(now)
    }
}

/// How long a cycle lasts, in seconds.
const CYCLE: u64 = 20;
/// How much of each cycle the connection is down, in seconds.
const DOWN: u64 = 6;

/// Whether the connection counts as down at `now`.
#[must_use]
fn is_down(now: SystemTime) -> bool {
    now.duration_since(SystemTime::UNIX_EPOCH)
        .is_ok_and(|since| since.as_secs() % CYCLE < DOWN)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn down_six_seconds_of_every_twenty() {
        let at = |seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
        let down: Vec<u64> = (0..40).filter(|&s| is_down(at(s))).collect();
        assert_eq!(down, [0, 1, 2, 3, 4, 5, 20, 21, 22, 23, 24, 25]);
        assert!(Mode::Flaky.drops_at(at(3)) && !Mode::Flaky.drops_at(at(7)));
        assert!(!Mode::Off.drops_at(at(3)));
    }
}
