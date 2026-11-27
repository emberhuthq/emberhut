//! The owner's calendar, as their Mac's calendars give it: the rules for a range of days, an
//! event's id and day, and what kind of time it is (a meeting with others, an event of their
//! own, or focus time they or the assistant blocked).
//!
//! Days are calendar days (`2026-10-19`) in the server's time zone; the caller passes the zone.

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};
use sha2::{Digest as _, Sha256};

/// The most days one look at the calendar covers.
pub const MAX_DAYS: i64 = 62;

/// Why a range of days is not one the calendar takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RangeError {
    /// A bound is not a day, or `to` is before `from`.
    #[error("from and to must be dates like 2026-10-19, to on or after from")]
    NotDays,
    /// More than [`MAX_DAYS`] days apart.
    #[error("at most 62 days at a time")]
    TooLong,
}

/// `text` as a day, when it is exactly `YYYY-MM-DD` and a real date.
#[must_use]
pub fn day(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    text.parse().ok()
}

/// A day as the wire writes it: `2026-10-19`.
#[must_use]
pub fn day_text(day: Date) -> String {
    day.strftime("%Y-%m-%d").to_string()
}

/// The range `from..=to` the calendar takes: two days, `to` not before `from`, at most
/// [`MAX_DAYS`] apart.
///
/// # Errors
///
/// [`RangeError`].
pub fn range(from: &str, to: &str) -> Result<(Date, Date), RangeError> {
    let (Some(from), Some(to)) = (day(from), day(to)) else {
        return Err(RangeError::NotDays);
    };
    if to < from {
        return Err(RangeError::NotDays);
    }
    if days_between(from, to) > MAX_DAYS {
        return Err(RangeError::TooLong);
    }
    Ok((from, to))
}

/// Whole days from `from` to `to` (negative when `to` is earlier).
#[must_use]
pub fn days_between(from: Date, to: Date) -> i64 {
    from.until(to).map_or(0, |span| i64::from(span.get_days()))
}

/// The day after `day` (`day` itself at the end of time).
#[must_use]
pub fn next_day(day: Date) -> Date {
    day.checked_add(1.day()).unwrap_or(day)
}

/// The instant `text` names: a time with an offset, a time without one (in `zone`), or a day
/// alone (its midnight UTC, as JavaScript reads one).
#[must_use]
pub fn instant(text: &str, zone: &TimeZone) -> Option<Timestamp> {
    let text = text.trim();
    if let Ok(at) = text.parse::<Timestamp>() {
        return Some(at);
    }
    if let Some(date) = day(text) {
        return TimeZone::UTC
            .to_timestamp(date.to_datetime(jiff::civil::Time::midnight()))
            .ok();
    }
    let local: DateTime = text.parse().ok()?;
    zone.to_timestamp(local).ok()
}

/// The day `text` falls on in `zone` (a day alone is that day).
#[must_use]
pub fn day_of(text: &str, zone: &TimeZone) -> Option<Date> {
    if let Some(date) = day(text.trim()) {
        return Some(date);
    }
    instant(text, zone).map(|at| zone.to_datetime(at).date())
}

/// The last day an event that ends at `end` is on: the day of its end, or of the moment
/// before it for an all-day event (which ends at the next midnight).
#[must_use]
pub fn last_day(end: &str, all_day: bool, zone: &TimeZone) -> Option<Date> {
    let at = instant(end, zone)?;
    let at = if all_day {
        at.checked_sub(1.millisecond()).ok()?
    } else {
        at
    };
    Some(zone.to_datetime(at).date())
}

/// An event's id: the first 16 hex digits of the SHA-256 of its calendar, title, start and end
/// (each as the Mac wrote it), so the same event gets the same id every time it is read.
#[must_use]
pub fn event_id(calendar: &str, title: &str, start: &str, end: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(format!("{calendar}|{title}|{start}|{end}").as_bytes());
    digest[..8]
        .iter()
        .flat_map(|b| [HEX[usize::from(b >> 4)], HEX[usize::from(b & 0xf)]])
        .map(char::from)
        .collect()
}

/// What kind of time an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Someone else is in it.
    Meeting,
    /// Only the owner.
    Event,
    /// Time blocked to work: by the assistant (`add_event`), or titled as such.
    Focus,
}

impl EventKind {
    /// The kind's word on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Meeting => "meeting",
            Self::Event => "event",
            Self::Focus => "focus",
        }
    }
}

/// The words a focus block's title starts with.
const FOCUS_WORDS: &[&str] = &["deep work", "focus", "study", "block"];

/// Whether `title` names focus time: it starts with one of `FOCUS_WORDS` (any case), as a
/// whole word.
#[must_use]
pub fn titled_focus(title: &str) -> bool {
    let lower = title.to_lowercase();
    FOCUS_WORDS.iter().any(|word| {
        lower.strip_prefix(word).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        })
    })
}

/// The key a block the assistant put on the calendar is remembered by: its title and its
/// start as an instant (`title|2026-10-19T09:00:00.000Z`).
#[must_use]
pub fn block_key(title: &str, start: Timestamp) -> String {
    format!("{title}|{}", start.strftime("%Y-%m-%dT%H:%M:%S%.3fZ"))
}

/// An event's kind: focus when it is a remembered block or titled as focus time, a meeting
/// when more than one person is in it, an event otherwise.
#[must_use]
pub fn event_kind(
    title: &str,
    start: Option<Timestamp>,
    people: f64,
    is_block: impl Fn(&str) -> bool,
) -> EventKind {
    let block = start.is_some_and(|start| is_block(&block_key(title, start)));
    if block || titled_focus(title) {
        EventKind::Focus
    } else if people > 1.0 {
        EventKind::Meeting
    } else {
        EventKind::Event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Date {
        day(text).unwrap()
    }

    #[test]
    fn a_range_is_two_days_in_order_and_at_most_62_apart() {
        assert_eq!(
            range("2026-10-19", "2026-10-25"),
            Ok((d("2026-10-19"), d("2026-10-25")))
        );
        assert_eq!(range("2026-10-25", "2026-10-19"), Err(RangeError::NotDays));
        assert_eq!(range("2026-10-1", "2026-10-19"), Err(RangeError::NotDays));
        assert_eq!(range("2026-02-30", "2026-03-01"), Err(RangeError::NotDays));
        assert_eq!(
            range("2026-01-01", "2026-03-04"),
            Ok((d("2026-01-01"), d("2026-03-04")))
        );
        assert_eq!(range("2026-01-01", "2026-03-05"), Err(RangeError::TooLong));
    }

    #[test]
    fn an_events_days_are_read_in_the_zone() {
        let utc = TimeZone::UTC;
        let plus3 = TimeZone::fixed(jiff::tz::offset(3));
        assert_eq!(day_of("2026-10-19T22:30:00Z", &utc), Some(d("2026-10-19")));
        assert_eq!(
            day_of("2026-10-19T22:30:00Z", &plus3),
            Some(d("2026-10-20"))
        );
        assert_eq!(day_of("2026-10-19", &plus3), Some(d("2026-10-19")));
        assert_eq!(
            last_day("2026-10-21T00:00:00Z", true, &utc),
            Some(d("2026-10-20")),
            "an all-day event ends at the next midnight"
        );
        assert_eq!(
            last_day("2026-10-21T00:00:00Z", false, &utc),
            Some(d("2026-10-21"))
        );
        assert_eq!(day_of("soon", &utc), None);
    }

    #[test]
    fn ids_are_stable_and_short() {
        let id = event_id(
            "Work",
            "Standup",
            "2026-10-19T09:00:00Z",
            "2026-10-19T09:15:00Z",
        );
        assert_eq!(id.len(), 16);
        assert_eq!(
            id,
            event_id(
                "Work",
                "Standup",
                "2026-10-19T09:00:00Z",
                "2026-10-19T09:15:00Z"
            )
        );
        assert_ne!(
            id,
            event_id(
                "Home",
                "Standup",
                "2026-10-19T09:00:00Z",
                "2026-10-19T09:15:00Z"
            )
        );
    }

    #[test]
    fn focus_meeting_or_event() {
        let start: Timestamp = "2026-10-19T09:00:00Z".parse().unwrap();
        let none = |_: &str| false;
        assert_eq!(
            event_kind("Deep work: draft", Some(start), 1.0, none),
            EventKind::Focus
        );
        assert_eq!(
            event_kind("Focusing", Some(start), 1.0, none),
            EventKind::Event
        );
        assert_eq!(
            event_kind("Weekly", Some(start), 3.0, none),
            EventKind::Meeting
        );
        assert_eq!(
            event_kind("Dentist", Some(start), 0.0, none),
            EventKind::Event
        );
        let blocked = |key: &str| key == "Write the report|2026-10-19T09:00:00.000Z";
        assert_eq!(
            event_kind("Write the report", Some(start), 1.0, blocked),
            EventKind::Focus
        );
    }

    #[test]
    fn days_step_and_count() {
        assert_eq!(next_day(d("2026-10-31")), d("2026-11-01"));
        assert_eq!(days_between(d("2026-10-19"), d("2026-10-25")), 6);
        assert_eq!(days_between(d("2026-10-25"), d("2026-10-19")), -6);
        assert_eq!(day_text(d("2026-01-05")), "2026-01-05");
    }
}
