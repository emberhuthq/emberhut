//! An agent's spending: what its approved payments add up to in a calendar month, against its
//! cap. The cap can only refuse a payment, never approve one: a payment is the fixed core's,
//! and the owner approves it.

use std::fmt;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::agents::SpendCap;

/// A calendar month, `YYYY-MM`, as spending is summed by.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonthKey(String);

impl MonthKey {
    /// The month `at` falls in, in `zone` (the server's time zone).
    ///
    /// ```
    /// use jiff::Timestamp;
    /// use jiff::tz::TimeZone;
    /// use emberhut_core::approvals::MonthKey;
    ///
    /// let at: Timestamp = "2026-11-30T23:30:00Z".parse().unwrap();
    /// assert_eq!(MonthKey::of(at, &TimeZone::UTC).as_str(), "2026-11");
    /// assert_eq!(MonthKey::of(at, &TimeZone::fixed(jiff::tz::offset(2))).as_str(), "2026-12");
    /// ```
    #[must_use]
    pub fn of(at: Timestamp, zone: &TimeZone) -> Self {
        let date = zone.to_datetime(at).date();
        Self(format!("{:04}-{:02}", date.year(), date.month()))
    }

    /// The month stored as `text`, if it is `YYYY-MM`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (year, month) = text.split_once('-')?;
        let fits = year.len() == 4
            && year.bytes().all(|b| b.is_ascii_digit())
            && month.len() == 2
            && month.parse::<u8>().is_ok_and(|m| (1..=12).contains(&m));
        fits.then(|| Self(text.to_owned()))
    }

    /// The month as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MonthKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a payment of `amount_cents` keeps the month's spending, `spent_cents` so far,
/// within `cap`. No cap: no spending at all.
///
/// ```
/// use emberhut_core::agents::{Currency, SpendCap};
/// use emberhut_core::approvals::within_cap;
///
/// let cap = SpendCap { cents: 10_000, currency: Currency::parse("EUR").unwrap() };
/// assert!(within_cap(9_000, 1_000, Some(&cap)));
/// assert!(!within_cap(9_000, 1_001, Some(&cap)));
/// assert!(!within_cap(0, 1, None));
/// ```
#[must_use]
pub fn within_cap(spent_cents: u64, amount_cents: u64, cap: Option<&SpendCap>) -> bool {
    cap.is_some_and(|cap| {
        spent_cents
            .checked_add(amount_cents)
            .is_some_and(|total| total <= cap.cents)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Currency;

    fn cap(cents: u64) -> SpendCap {
        SpendCap {
            cents,
            currency: Currency::parse("EUR").unwrap(),
        }
    }

    #[test]
    fn the_cap_refuses_and_nothing_else() {
        assert!(within_cap(0, 10_000, Some(&cap(10_000))));
        assert!(!within_cap(1, 10_000, Some(&cap(10_000))));
        assert!(within_cap(0, 0, Some(&cap(0))));
        assert!(!within_cap(0, 1, Some(&cap(0))));
        assert!(!within_cap(0, 0, None), "no cap: no spending");
        assert!(!within_cap(u64::MAX, 1, Some(&cap(u64::MAX))));
    }

    #[test]
    fn months() {
        let at: Timestamp = "2027-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(MonthKey::of(at, &TimeZone::UTC).as_str(), "2027-01");
        assert_eq!(
            MonthKey::of(at, &TimeZone::fixed(jiff::tz::offset(-1))).as_str(),
            "2026-12"
        );
        assert_eq!(MonthKey::parse("2026-11").unwrap().to_string(), "2026-11");
        for bad in ["2026-13", "2026-1", "26-11", "2026/11", ""] {
            assert_eq!(MonthKey::parse(bad), None, "{bad}");
        }
    }
}
