//! The days the calendar shows: one day, its week (Monday first), or its month's whole weeks;
//! moving by a day, a week or a month; the title over them; and whether Today would change
//! anything. Pure, over civil dates, so it is tested without a clock.

use jiff::ToSpan as _;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// What the calendar shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Mode {
    /// One day.
    Day,
    /// A week.
    #[default]
    Week,
    /// A month.
    Month,
}

impl Mode {
    /// Every mode, as the picker lists them.
    pub(crate) const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    /// Its name on the picker.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
        }
    }
}

/// The Monday of `day`'s week.
fn week_start(day: Date) -> Date {
    let back = i64::from(day.weekday().to_monday_zero_offset());
    day.checked_sub(back.days()).unwrap_or(day)
}

/// The days shown for `mode` around `anchor`, first to last.
#[must_use]
pub(crate) fn days(mode: Mode, anchor: Date) -> Vec<Date> {
    let (start, count) = match mode {
        Mode::Day => (anchor, 1),
        Mode::Week => (week_start(anchor), 7),
        Mode::Month => {
            let first = anchor.first_of_month();
            let start = week_start(first);
            let last = anchor.last_of_month();
            let end = week_start(last).checked_add(6.days()).unwrap_or(last);
            let count = start.until(end).map_or(35, |span| span.get_days() + 1);
            (start, count)
        }
    };
    (0..count)
        .filter_map(|n| start.checked_add(i64::from(n).days()).ok())
        .collect()
}

/// `anchor` moved `by` days, weeks or months (back when negative).
#[must_use]
pub(crate) fn step(mode: Mode, anchor: Date, by: i64) -> Date {
    let moved = match mode {
        Mode::Day => anchor.checked_add(by.days()),
        Mode::Week => anchor.checked_add((7 * by).days()),
        Mode::Month => anchor.checked_add(by.months()),
    };
    moved.unwrap_or(anchor)
}

/// The title over the days: `Tuesday 24 November` (Day), `November 2026` or `Nov – Dec 2026`
/// (Week), `November 2026` (Month).
#[must_use]
pub(crate) fn title(mode: Mode, anchor: Date) -> String {
    match mode {
        Mode::Day => anchor.strftime("%A %-d %B").to_string(),
        Mode::Week => {
            let shown = days(mode, anchor);
            let (first, last) = (
                shown.first().copied().unwrap_or(anchor),
                shown.last().copied().unwrap_or(anchor),
            );
            if (first.year(), first.month()) == (last.year(), last.month()) {
                first.strftime("%B %Y").to_string()
            } else {
                format!("{} – {}", first.strftime("%b"), last.strftime("%b %Y"))
            }
        }
        Mode::Month => anchor.strftime("%B %Y").to_string(),
    }
}

/// Whether Today would move anything: off when today is already shown (in Month, when the
/// month is today's).
#[must_use]
pub(crate) fn shows_today(mode: Mode, anchor: Date, today: Date) -> bool {
    match mode {
        Mode::Month => (anchor.year(), anchor.month()) == (today.year(), today.month()),
        Mode::Day | Mode::Week => days(mode, anchor).contains(&today),
    }
}

/// The step a drag across the days makes, `dx` and `dy` pixels: the next days for a drag to
/// the left, the previous ones to the right, once it is far enough (70 px) and more sideways
/// than up or down.
#[must_use]
pub(crate) fn drag_step(dx: f32, dy: f32) -> Option<i64> {
    (dx.abs() > 70. && dx.abs() > dy.abs() * 1.6).then_some(if dx < 0. { 1 } else { -1 })
}

/// The weekday names over Month's columns, Monday first.
pub(crate) const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[cfg(test)]
mod tests {
    use jiff::civil::{Weekday, date};

    use super::*;

    #[test]
    fn a_week_runs_monday_to_sunday() {
        let shown = days(Mode::Week, date(2026, 11, 26));
        assert_eq!(shown.len(), 7);
        assert_eq!(shown[0], date(2026, 11, 23));
        assert_eq!(shown[6], date(2026, 11, 29));
        assert_eq!(days(Mode::Day, date(2026, 11, 26)), [date(2026, 11, 26)]);
    }

    #[test]
    fn a_month_is_its_whole_weeks() {
        let shown = days(Mode::Month, date(2026, 11, 26));
        assert_eq!(shown.first(), Some(&date(2026, 10, 26)));
        assert_eq!(shown.last(), Some(&date(2026, 12, 6)));
        assert_eq!(shown.len() % 7, 0);
        assert_eq!(
            days(Mode::Month, date(2021, 2, 10)).len(),
            28,
            "a month of four whole weeks"
        );
    }

    #[test]
    fn moving_goes_by_the_modes_step() {
        let day = date(2026, 1, 31);
        assert_eq!(step(Mode::Day, day, 1), date(2026, 2, 1));
        assert_eq!(step(Mode::Week, day, -1), date(2026, 1, 24));
        assert_eq!(
            step(Mode::Month, day, 1),
            date(2026, 2, 28),
            "the month's last day"
        );
    }

    #[test]
    fn the_title_names_the_days_shown() {
        assert_eq!(title(Mode::Day, date(2026, 11, 24)), "Tuesday 24 November");
        assert_eq!(title(Mode::Week, date(2026, 11, 24)), "November 2026");
        assert_eq!(title(Mode::Week, date(2026, 12, 1)), "Nov – Dec 2026");
        assert_eq!(title(Mode::Month, date(2026, 11, 24)), "November 2026");
    }

    #[test]
    fn a_long_flat_drag_moves() {
        assert_eq!(drag_step(-120., 10.), Some(1), "to the left: the next days");
        assert_eq!(drag_step(90., -20.), Some(-1));
        assert_eq!(drag_step(50., 0.), None, "too short");
        assert_eq!(drag_step(100., 80.), None, "too steep");
    }

    #[test]
    fn today_is_off_while_today_shows() {
        let today = date(2026, 11, 26);
        assert!(shows_today(Mode::Week, date(2026, 11, 23), today));
        assert!(!shows_today(Mode::Week, date(2026, 11, 30), today));
        assert!(shows_today(Mode::Month, date(2026, 11, 1), today));
        assert!(!shows_today(Mode::Day, date(2026, 11, 25), today));
        let first = days(Mode::Month, today)[0];
        assert_eq!(first.weekday(), Weekday::Monday, "under WEEKDAYS[0]");
        assert_eq!(WEEKDAYS[0], "Mon");
    }
}
