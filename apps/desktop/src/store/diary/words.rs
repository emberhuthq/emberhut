//! How the diary words its days and times: `Today`, `Yesterday`, a weekday within the week,
//! `Saturday 25 October` this year, `3 March 2025` before; a note's time as `7:15 AM`. Days and
//! times are on this device's clock ([`zone`]); a synced note keeps the day the server filed it
//! under. Everything takes `now`, so it is tested without a clock.

use emberhut_api::notes::Note;
use emberhut_api::time::Day;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ZonedDateTime};

use crate::store::tasks::clock::{clock_minutes, instant};

/// The zone the diary's days and times are in: this device's, as the calendar and the chat
/// show times. A note not sent yet takes its day in it until the server files it.
#[must_use]
pub(crate) fn zone() -> TimeZone {
    TimeZone::system()
}

/// Today, in `zone`.
#[must_use]
pub(crate) fn today(now: Timestamp, zone: &TimeZone) -> Day {
    super::book::day_of(now, zone)
}

/// A day's title in the diary.
#[must_use]
pub(crate) fn title(day: &Day, now: &ZonedDateTime) -> String {
    let today = now.date();
    let Ok(date) = day.as_str().parse::<Date>() else {
        return day.to_string();
    };
    if date == today {
        return "Today".to_owned();
    }
    if today.yesterday().ok() == Some(date) {
        return "Yesterday".to_owned();
    }
    let back = date
        .until(today)
        .map_or(i64::MAX, |span| i64::from(span.get_days()));
    if (0..6).contains(&back) {
        date.strftime("%A").to_string()
    } else if date.year() == today.year() {
        date.strftime("%A %-d %B").to_string()
    } else {
        date.strftime("%-d %B %Y").to_string()
    }
}

/// A note's time in `zone`: `7:15 AM`.
#[must_use]
pub(crate) fn time(note: &Note, zone: &TimeZone) -> String {
    instant(&note.at).map_or_default(|at| clock_minutes(&at.to_zoned_datetime(zone.clone())))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn days_read_as_people_say_them() {
        // Tuesday 24 November 2026.
        let now = "2026-11-24T09:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC);
        let title = |day: &str| title(&Day::new(day), &now);
        assert_eq!(title("2026-11-24"), "Today");
        assert_eq!(title("2026-11-23"), "Yesterday");
        assert_eq!(title("2026-11-19"), "Thursday");
        assert_eq!(title("2026-11-18"), "Wednesday 18 November");
        assert_eq!(title("2025-03-03"), "3 March 2025");
        assert_eq!(title("not a day"), "not a day");
    }

    #[test]
    fn a_notes_time_is_in_the_zone() {
        let note: Note = serde_json::from_value(json!({
            "uid": "aaaaaaaa", "who": "owner", "day": "2026-11-24", "at": "2026-11-24T04:15:00.000Z",
            "text": "x", "created": "2026-11-24T04:15:00.000Z", "updated": "2026-11-24T04:15:00.000Z"
        }))
        .unwrap();
        let zone = TimeZone::fixed(jiff::tz::offset(3));
        assert_eq!(time(&note, &zone), "7:15 AM");
        assert_eq!(
            today("2026-11-24T22:00:00Z".parse().unwrap(), &zone).as_str(),
            "2026-11-25"
        );
    }
}
