//! How instants are stored: whole milliseconds since the Unix epoch, as an integer.
//!
//! Integers compare and sort exactly in SQL, where text timestamps only do while every row is
//! written with the same precision and offset.

use jiff::Timestamp;

use crate::DbError;

/// `instant` as a column stores it.
pub(crate) fn millis(instant: Timestamp) -> i64 {
    instant.as_millisecond()
}

/// The instant a column stores as `millis`.
pub(crate) fn instant(millis: i64) -> Result<Timestamp, DbError> {
    Timestamp::from_millisecond(millis).map_err(|_| DbError::InstantOutOfRange(millis))
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    #[test]
    fn keeps_whole_milliseconds() {
        let at = Timestamp::UNIX_EPOCH + SignedDuration::from_micros(1_500_999);
        assert_eq!(millis(at), 1_500);
        assert_eq!(
            instant(1_500).unwrap(),
            Timestamp::UNIX_EPOCH + SignedDuration::from_millis(1_500)
        );
    }

    #[test]
    fn out_of_range_is_an_error() {
        assert!(matches!(
            instant(i64::MAX),
            Err(DbError::InstantOutOfRange(i64::MAX))
        ));
    }
}
