//! How times read on a task's page, worded from `now` and a time zone passed in, so they are
//! tested without a clock: `5 minutes ago`, `Today 9:12 AM`, `around 12:20 AM`, `2h 5m`.

use jiff::{Timestamp, ToSpan as _, ZonedDateTime};

/// A contract time stamp as an instant.
#[must_use]
pub(crate) fn instant(at: &emberhut_api::time::Timestamp) -> Option<Timestamp> {
    at.as_str().parse().ok()
}

/// `n unit` or `n units`.
fn plural(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// How far `at` is from `now`, as people say it: `now`, `5 minutes ago`, `yesterday`,
/// `in 2 hours`, `3 weeks ago`.
#[must_use]
pub(crate) fn relative(at: Timestamp, now: Timestamp) -> String {
    let seconds = now.as_second() - at.as_second();
    let ahead = seconds < 0;
    let gap = seconds.abs();
    let words = match gap {
        0..45 => return "now".to_owned(),
        45..3600 => plural((gap / 60).max(1), "minute"),
        3600..86_400 => plural(gap / 3600, "hour"),
        86_400..172_800 => return if ahead { "tomorrow" } else { "yesterday" }.to_owned(),
        172_800..604_800 => plural(gap / 86_400, "day"),
        _ => plural(gap / 604_800, "week"),
    };
    if ahead {
        format!("in {words}")
    } else {
        format!("{words} ago")
    }
}

/// A clock time: `9:12 AM`, or `9 AM` on the hour.
#[must_use]
pub(crate) fn clock(at: &ZonedDateTime) -> String {
    if at.minute() == 0 {
        at.strftime("%-I %p").to_string()
    } else {
        at.strftime("%-I:%M %p").to_string()
    }
}

/// A clock time with its minutes: `9:12 AM`, `9:00 AM`.
#[must_use]
pub(crate) fn clock_minutes(at: &ZonedDateTime) -> String {
    at.strftime("%-I:%M %p").to_string()
}

/// `Today 9:12 AM`, else `Thu 9:12 AM`.
#[must_use]
pub(crate) fn day_and_time(at: Timestamp, now: &ZonedDateTime) -> String {
    let at = at.to_zoned_datetime(now.time_zone().clone());
    if at.date() == now.date() {
        format!("Today {}", at.strftime("%-I:%M %p"))
    } else {
        at.strftime("%a %-I:%M %p").to_string()
    }
}

/// `9:12 AM` within 12 hours of `now`, else `tomorrow 9 AM`, `yesterday 8:50 PM`, `Thu 9 AM`.
#[must_use]
pub(crate) fn said(at: Timestamp, now: &ZonedDateTime) -> String {
    let zoned = at.to_zoned_datetime(now.time_zone().clone());
    let time = clock(&zoned);
    if (at.as_second() - now.timestamp().as_second()).abs() < 12 * 3600 {
        return time;
    }
    let today = now.date();
    if today.tomorrow().ok() == Some(zoned.date()) {
        format!("tomorrow {time}")
    } else if today.yesterday().ok() == Some(zoned.date()) {
        format!("yesterday {time}")
    } else {
        format!("{} {time}", zoned.strftime("%a"))
    }
}

/// `around 12:20 AM`: [`said`], to the nearest five minutes.
#[must_use]
pub(crate) fn around(at: Timestamp, now: &ZonedDateTime) -> String {
    let rounded = (at.as_second() + 150).div_euclid(300) * 300;
    let at = Timestamp::from_second(rounded).unwrap_or(at);
    format!("around {}", said(at, now))
}

/// How often, in minutes: `4 h`, `1 h 30 min`, `30 min`, `day`, `2 days`, kept on one line.
#[must_use]
pub(crate) fn interval(minutes: i64) -> String {
    let words = if minutes > 0 && minutes % 1440 == 0 {
        if minutes == 1440 {
            "day".to_owned()
        } else {
            format!("{} days", minutes / 1440)
        }
    } else if minutes < 60 {
        format!("{minutes} min")
    } else if minutes % 60 == 0 {
        format!("{} h", minutes / 60)
    } else {
        format!("{} h {} min", minutes / 60, minutes % 60)
    };
    words.replace(' ', "\u{00A0}")
}

/// How long from `from` to `to`, in at most two units: `45s`, `28m`, `2h 5m`, `3d 4h`.
#[must_use]
pub(crate) fn span(from: Timestamp, to: Timestamp) -> String {
    let seconds = (to.as_second() - from.as_second()).max(1);
    match seconds {
        ..60 => format!("{seconds}s"),
        60..3600 => {
            let (m, s) = (seconds / 60, seconds % 60);
            if s == 0 || m >= 10 {
                format!("{m}m")
            } else {
                format!("{m}m {s}s")
            }
        }
        3600..86_400 => {
            let (h, m) = (seconds / 3600, seconds % 3600 / 60);
            if m == 0 {
                format!("{h}h")
            } else {
                format!("{h}h {m}m")
            }
        }
        _ => {
            let (d, h) = (seconds / 86_400, seconds % 86_400 / 3600);
            if h == 0 {
                format!("{d}d")
            } else {
                format!("{d}d {h}h")
            }
        }
    }
}

/// The start of the day a week from `now`'s (where Not now puts a task off until).
#[must_use]
pub(crate) fn next_week(now: &ZonedDateTime) -> Timestamp {
    now.date()
        .checked_add(7.days())
        .ok()
        .and_then(|day| day.to_zoned_datetime(now.time_zone().clone()).ok())
        .map_or_else(|| now.timestamp(), |day| day.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> ZonedDateTime {
        "2026-11-25T10:00:00+02:00[+02:00]".parse().unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn times_read_as_people_say_them() {
        let now = now().timestamp();
        assert_eq!(relative(at("2026-11-25T07:59:50Z"), now), "now");
        assert_eq!(relative(at("2026-11-25T07:55:00Z"), now), "5 minutes ago");
        assert_eq!(relative(at("2026-11-25T06:00:00Z"), now), "2 hours ago");
        assert_eq!(relative(at("2026-11-24T06:00:00Z"), now), "yesterday");
        assert_eq!(relative(at("2026-11-25T09:00:00Z"), now), "in 1 hour");
        assert_eq!(relative(at("2026-11-04T08:00:00Z"), now), "3 weeks ago");
    }

    #[test]
    fn a_day_and_a_time_read_in_the_local_zone() {
        let now = now();
        assert_eq!(
            day_and_time(at("2026-11-25T07:12:00Z"), &now),
            "Today 9:12 AM"
        );
        assert_eq!(
            day_and_time(at("2026-11-19T18:40:00Z"), &now),
            "Thu 8:40 PM"
        );
        assert_eq!(said(at("2026-11-25T18:50:00Z"), &now), "8:50 PM");
        assert_eq!(said(at("2026-11-26T22:00:00Z"), &now), "Fri 12 AM");
        assert_eq!(said(at("2026-11-26T07:00:00Z"), &now), "tomorrow 9 AM");
        assert_eq!(around(at("2026-11-25T17:21:40Z"), &now), "around 7:20 PM");
    }

    #[test]
    fn lengths_and_intervals_keep_to_two_units() {
        assert_eq!(
            span(at("2026-11-25T08:00:00Z"), at("2026-11-25T08:00:45Z")),
            "45s"
        );
        assert_eq!(
            span(at("2026-11-25T08:00:00Z"), at("2026-11-25T08:28:00Z")),
            "28m"
        );
        assert_eq!(
            span(at("2026-11-25T08:00:00Z"), at("2026-11-25T10:05:00Z")),
            "2h 5m"
        );
        assert_eq!(interval(240), "4\u{a0}h");
        assert_eq!(interval(90), "1\u{a0}h\u{a0}30\u{a0}min");
        assert_eq!(interval(1440), "day");
        assert_eq!(interval(30), "30\u{a0}min");
    }

    #[test]
    fn next_week_starts_a_week_from_today() {
        assert_eq!(
            next_week(&now()),
            at("2026-12-01T22:00:00Z"),
            "midnight on 2 Dec, two hours east of UTC"
        );
    }
}
