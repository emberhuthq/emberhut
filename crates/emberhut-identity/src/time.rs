//! Times as statements and the relay's messages carry them: whole milliseconds since the Unix
//! epoch.

use std::fmt;
use std::time::{Duration, SystemTime};

/// An instant as whole milliseconds since the Unix epoch, as a JSON number on the wire and 8
/// big-endian bytes in a statement.
///
/// It is at most 2^53 − 1 ([`UnixMillis::MAX`]), so a JavaScript number holds it exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "u64", into = "u64")
)]
pub struct UnixMillis(u64);

/// A time before the Unix epoch, or after [`UnixMillis::MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("the time is outside 0 to 2^53 - 1 milliseconds since the Unix epoch")]
pub struct TimeOutOfRange;

impl UnixMillis {
    /// The latest time: 2^53 − 1 milliseconds, the largest integer a JavaScript number holds
    /// exactly.
    pub const MAX: Self = Self((1 << 53) - 1);

    /// The instant `millis` milliseconds after the epoch.
    ///
    /// # Errors
    ///
    /// [`TimeOutOfRange`] above [`UnixMillis::MAX`].
    pub const fn new(millis: u64) -> Result<Self, TimeOutOfRange> {
        if millis > Self::MAX.0 {
            return Err(TimeOutOfRange);
        }
        Ok(Self(millis))
    }

    /// Milliseconds since the epoch.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// How far apart two instants are, whichever comes first: for checks like "within 5
    /// minutes of the clock".
    #[must_use]
    pub const fn distance(self, other: Self) -> Duration {
        Duration::from_millis(self.0.abs_diff(other.0))
    }
}

impl TryFrom<u64> for UnixMillis {
    type Error = TimeOutOfRange;

    fn try_from(millis: u64) -> Result<Self, Self::Error> {
        Self::new(millis)
    }
}

impl From<UnixMillis> for u64 {
    fn from(time: UnixMillis) -> Self {
        time.0
    }
}

impl TryFrom<SystemTime> for UnixMillis {
    type Error = TimeOutOfRange;

    /// The system time, rounded down to the millisecond.
    fn try_from(time: SystemTime) -> Result<Self, Self::Error> {
        let since = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| TimeOutOfRange)?;
        let millis = u64::try_from(since.as_millis()).map_err(|_| TimeOutOfRange)?;
        Self::new(millis)
    }
}

impl From<UnixMillis> for SystemTime {
    fn from(time: UnixMillis) -> Self {
        SystemTime::UNIX_EPOCH + Duration::from_millis(time.0)
    }
}

impl fmt::Display for UnixMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_from_and_to_system_time() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_micros(1_795_561_073_000_999);
        let millis = UnixMillis::try_from(time).unwrap();
        assert_eq!(millis, UnixMillis::new(1_795_561_073_000).unwrap());
        assert_eq!(
            SystemTime::from(millis),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_795_561_073)
        );
        assert_eq!(millis.to_string(), "1795561073000");
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(UnixMillis::try_from(before), Err(TimeOutOfRange));
    }

    #[test]
    fn times_fit_a_javascript_number() {
        assert_eq!(UnixMillis::new(9_007_199_254_740_991), Ok(UnixMillis::MAX));
        assert_eq!(UnixMillis::new(9_007_199_254_740_992), Err(TimeOutOfRange));
        assert_eq!(UnixMillis::try_from(u64::MAX), Err(TimeOutOfRange));
        assert_eq!(u64::from(UnixMillis::MAX), 9_007_199_254_740_991);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = UnixMillis::new(1_000).unwrap();
        let b = UnixMillis::new(301_000).unwrap();
        assert_eq!(a.distance(b), Duration::from_secs(300));
        assert_eq!(b.distance(a), a.distance(b));
    }
}
