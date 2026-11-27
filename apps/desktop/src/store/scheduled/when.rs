//! When a scheduled row runs next, as the page groups and words it, in the user's time zone:
//! the group (Next hour, Today, Tomorrow, This week, This month, Later, No next run), the
//! "Runs" filter's spans, and the row's time (`8:30 AM`, `Tue 8:30 PM`, `Nov 20`) with what is
//! left (`in 2h 5m`). Everything takes `now`, so it is tested without a clock.

use jiff::{Timestamp, ZonedDateTime};

use crate::store::tasks::clock::clock_minutes;

/// A group of the page, by when its rows run next, soonest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NextRun {
    /// Within the hour (or overdue).
    Hour,
    /// Later today.
    Today,
    /// Tomorrow.
    Tomorrow,
    /// Within six days.
    Week,
    /// Within thirty days.
    Month,
    /// Further off.
    Later,
    /// No next run.
    None,
}

impl NextRun {
    /// Every group, in the page's order.
    pub(crate) const ALL: [Self; 7] = [
        Self::Hour,
        Self::Today,
        Self::Tomorrow,
        Self::Week,
        Self::Month,
        Self::Later,
        Self::None,
    ];

    /// The group's header.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Hour => "Next hour",
            Self::Today => "Today",
            Self::Tomorrow => "Tomorrow",
            Self::Week => "This week",
            Self::Month => "This month",
            Self::Later => "Later",
            Self::None => "No next run",
        }
    }

    /// The group a row running next at `next` is in.
    #[must_use]
    pub(crate) fn of(next: Option<Timestamp>, now: &ZonedDateTime) -> Self {
        let Some(next) = next else {
            return Self::None;
        };
        if next.as_second() - now.timestamp().as_second() < 3600 {
            return Self::Hour;
        }
        match days_from_today(next, now) {
            0 => Self::Today,
            1 => Self::Tomorrow,
            2..=6 => Self::Week,
            7..=30 => Self::Month,
            _ => Self::Later,
        }
    }
}

/// The "Runs" filter: within today, the week or the month.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Runs {
    /// Today.
    Today,
    /// Within six days.
    ThisWeek,
    /// Within thirty days.
    ThisMonth,
}

impl Runs {
    /// Every choice, in the filter's order.
    pub(crate) const ALL: [Self; 3] = [Self::Today, Self::ThisWeek, Self::ThisMonth];

    /// The choice's name.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::ThisWeek => "This week",
            Self::ThisMonth => "This month",
        }
    }

    /// Whether a row running next at `next` runs within this span (one with no next run never
    /// does).
    #[must_use]
    pub(crate) fn takes(self, next: Option<Timestamp>, now: &ZonedDateTime) -> bool {
        let Some(next) = next else {
            return false;
        };
        let most = match self {
            Self::Today => 0,
            Self::ThisWeek => 6,
            Self::ThisMonth => 30,
        };
        days_from_today(next, now) <= most
    }
}

/// How many days `at` is after today, in `now`'s time zone (before today: negative).
fn days_from_today(at: Timestamp, now: &ZonedDateTime) -> i64 {
    let day = at.to_zoned_datetime(now.time_zone().clone()).date();
    now.date()
        .until(day)
        .map_or(0, |span| i64::from(span.get_days()))
}

/// The row's time: `8:30 AM` today or tomorrow (the group says which day), `Tue 8:30 PM` this
/// week, `Nov 20` further off.
#[must_use]
pub(crate) fn time(next: Timestamp, now: &ZonedDateTime) -> String {
    let at = next.to_zoned_datetime(now.time_zone().clone());
    match days_from_today(next, now) {
        0 | 1 => clock_minutes(&at),
        2..=6 => format!("{} {}", at.strftime("%a"), clock_minutes(&at)),
        _ => at.strftime("%b %-d").to_string(),
    }
}

/// What is left until `next` when it is under a day off (`now`, `in 5m`, `in 2h 5m`); none
/// further off.
#[must_use]
pub(crate) fn left(next: Timestamp, now: &ZonedDateTime) -> Option<String> {
    let seconds = next.as_second() - now.timestamp().as_second();
    match seconds {
        ..60 => Some("now".to_owned()),
        60..3600 => Some(format!("in {}m", seconds / 60)),
        3600..86_400 => Some(format!("in {}h {}m", seconds / 3600, seconds % 3600 / 60)),
        _ => None,
    }
}

/// Whether `next` is within the hour: what is left shows in the accent's colour.
#[must_use]
pub(crate) fn is_soon(next: Timestamp, now: &ZonedDateTime) -> bool {
    next.as_second() - now.timestamp().as_second() < 3600
}

/// The whole date and time, for the time's tip: `Tuesday 24 November 2026, 8:30 AM`.
#[must_use]
pub(crate) fn full(next: Timestamp, now: &ZonedDateTime) -> String {
    let at = next.to_zoned_datetime(now.time_zone().clone());
    format!("{}, {}", at.strftime("%A %-d %B %Y"), clock_minutes(&at))
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;

    use super::*;

    /// Tuesday 24 November 2026, 9:00 in UTC.
    fn now() -> ZonedDateTime {
        "2026-11-24T09:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC)
    }

    fn at(iso: &str) -> Timestamp {
        iso.parse().unwrap()
    }

    #[test]
    fn rows_group_by_how_soon_they_run() {
        let now = now();
        let group = |iso: &str| NextRun::of(Some(at(iso)), &now);
        assert_eq!(group("2026-11-24T09:30:00Z"), NextRun::Hour);
        assert_eq!(group("2026-11-24T08:00:00Z"), NextRun::Hour, "overdue");
        assert_eq!(group("2026-11-24T23:00:00Z"), NextRun::Today);
        assert_eq!(group("2026-11-25T08:30:00Z"), NextRun::Tomorrow);
        assert_eq!(group("2026-11-30T08:30:00Z"), NextRun::Week);
        assert_eq!(group("2026-12-20T08:30:00Z"), NextRun::Month);
        assert_eq!(group("2027-02-01T08:30:00Z"), NextRun::Later);
        assert_eq!(NextRun::of(None, &now), NextRun::None);
    }

    #[test]
    fn runs_takes_what_runs_within_its_span() {
        let now = now();
        let tomorrow = Some(at("2026-11-25T08:30:00Z"));
        assert!(!Runs::Today.takes(tomorrow, &now));
        assert!(Runs::ThisWeek.takes(tomorrow, &now));
        assert!(Runs::ThisMonth.takes(Some(at("2026-12-24T08:30:00Z")), &now));
        assert!(!Runs::ThisMonth.takes(Some(at("2026-12-25T08:30:00Z")), &now));
        assert!(!Runs::ThisMonth.takes(None, &now), "no next run");
    }

    #[test]
    fn the_time_says_as_much_as_the_group_doesnt() {
        let now = now();
        assert_eq!(time(at("2026-11-24T20:30:00Z"), &now), "8:30 PM");
        assert_eq!(time(at("2026-11-25T08:30:00Z"), &now), "8:30 AM");
        assert_eq!(time(at("2026-11-27T20:30:00Z"), &now), "Fri 8:30 PM");
        assert_eq!(time(at("2026-12-20T08:30:00Z"), &now), "Dec 20");
        assert_eq!(
            full(at("2026-11-24T20:30:00Z"), &now),
            "Tuesday 24 November 2026, 8:30 PM"
        );
    }

    #[test]
    fn what_is_left_shows_within_a_day() {
        let now = now();
        assert_eq!(
            left(at("2026-11-24T09:00:30Z"), &now).as_deref(),
            Some("now")
        );
        assert_eq!(
            left(at("2026-11-24T09:05:00Z"), &now).as_deref(),
            Some("in 5m")
        );
        assert_eq!(
            left(at("2026-11-24T11:05:00Z"), &now).as_deref(),
            Some("in 2h 5m")
        );
        assert_eq!(left(at("2026-11-25T09:00:00Z"), &now), None);
        assert!(is_soon(at("2026-11-24T09:59:00Z"), &now));
        assert!(!is_soon(at("2026-11-24T10:00:00Z"), &now));
    }
}
