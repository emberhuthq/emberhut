//! The wake rule: what the server notices becomes events, and the assistant decides. A burst
//! of events is batched into one wake of the room it is for; the assistant's own appointments
//! (a check-in, a watch's reminder) and news someone waits on wake it at once; and every room
//! together wakes at most [`MAX_WAKES_PER_HOUR`] times an hour, whatever happens.
//!
//! Also the times of day the assistant keeps: quiet hours, and daily routines.

use std::collections::VecDeque;

use jiff::civil::{Time, Weekday};
use jiff::{SignedDuration, Timestamp};

/// How long a burst of ordinary events may gather before the room wakes for them.
pub const BATCH: SignedDuration = SignedDuration::from_secs(90);

/// Ordinary events that wake the room at once when this many are waiting.
pub const BATCH_COUNT: usize = 10;

/// The waiting events one wake reads at most, oldest first ([`BATCH_COUNT`]): the rest wait
/// for the room's next wake, so a burst never makes one turn without bound.
pub const PER_WAKE: u64 = 10;

/// Wakes, every room's together, in any hour.
pub const MAX_WAKES_PER_HOUR: usize = 20;

/// The event kinds that wake their room at once: the assistant's own appointments, what
/// another room hands over, and news someone waits on.
pub const URGENT_KINDS: &[&str] = &[
    "checkin",
    "watch",
    "thread",
    "routine",
    "urgent",
    "ask",
    "important",
    "limits",
    "room",
];

/// Whether a room with waiting events of these `kinds`, the oldest told at `oldest`, wakes
/// now. Events of an urgent kind wake it at once; others once the oldest has waited [`BATCH`],
/// or once [`BATCH_COUNT`] are waiting.
#[must_use]
pub fn wakes_now(kinds: &[&str], oldest: Timestamp, now: Timestamp) -> bool {
    if kinds.is_empty() {
        return false;
    }
    kinds.iter().any(|kind| URGENT_KINDS.contains(kind))
        || now.duration_since(oldest) >= BATCH
        || kinds.len() >= BATCH_COUNT
}

/// The wakes of the last hour, every room's: the hourly ceiling.
#[derive(Clone, Debug, Default)]
pub struct WakeBudget {
    wakes: VecDeque<Timestamp>,
}

impl WakeBudget {
    /// Takes one wake at `now`; `false` when the last hour had [`MAX_WAKES_PER_HOUR`] already.
    pub fn take(&mut self, now: Timestamp) -> bool {
        while self
            .wakes
            .front()
            .is_some_and(|at| now.duration_since(*at) > SignedDuration::from_hours(1))
        {
            self.wakes.pop_front();
        }
        if self.wakes.len() >= MAX_WAKES_PER_HOUR {
            return false;
        }
        self.wakes.push_back(now);
        true
    }
}

/// One waiting event's line in the message that wakes its room: `- [kind] text`.
#[must_use]
pub fn event_line(kind: &str, text: &str) -> String {
    format!("- [{kind}] {text}")
}

/// What a wake from these event lines is about, for the room's status line while it runs:
/// one kind's own words, `<n> updates` for several, `Update` otherwise.
#[must_use]
pub fn about(lines: &str) -> String {
    let kinds: Vec<&str> = lines
        .lines()
        .filter_map(|line| {
            let kind = line.strip_prefix("- [")?;
            let end = kind.find(']')?;
            let kind = &kind[..end];
            (!kind.is_empty() && kind.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .then_some(kind)
        })
        .collect();
    match kinds.as_slice() {
        [one] => kind_words(one).unwrap_or("Update").to_owned(),
        [] => "Update".to_owned(),
        many => format!("{} updates", many.len()),
    }
}

/// An event kind in the status line's words.
fn kind_words(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "thread" => "Coding thread update",
        "check" => "Check",
        "checkin" => "Check-in",
        "routine" => "Routine",
        "plan" => "Plan",
        "ask" => "Your answer",
        "limits" => "Usage limits",
        "watch" => "Watch",
        "fact" => "Something learned",
        "secret" => "Saved secret",
        "stalled" => "A task stalled",
        "browser" => "Browser",
        "room" => "From another room",
        "messages" => "New messages",
        "important" => "A message that can't wait",
        "work" => "Work back",
        _ => return None,
    })
}

/// Quiet hours, as times of day (`from` up to `to`, which may be past midnight).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuietHours {
    /// When they start.
    pub from: Time,
    /// When they end.
    pub to: Time,
}

impl QuietHours {
    /// Whether `at` (a time of day) is inside them.
    #[must_use]
    pub fn contains(&self, at: Time) -> bool {
        if self.from > self.to {
            at >= self.from || at < self.to
        } else {
            at >= self.from && at < self.to
        }
    }
}

/// `HH:MM` as a time of day, the way routines and quiet hours are written; none for anything
/// else.
#[must_use]
pub fn hour_minute(text: &str) -> Option<Time> {
    let (hours, minutes) = text.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: i8 = hours.parse().ok()?;
    let minutes: i8 = minutes.parse().ok()?;
    Time::new(hours, minutes, 0, 0).ok()
}

/// Whether a routine at `at` (on `day`, or every day) is due at `now` (a local time of day on
/// a weekday): from its time on, the rest of that day.
#[must_use]
pub fn routine_due(at: Time, day: Option<Weekday>, now: Time, today: Weekday) -> bool {
    now >= at && day.is_none_or(|day| day == today)
}

/// A weekday as the routines write it: 0 Sunday to 6 Saturday.
#[must_use]
pub fn weekday_of(number: u8) -> Option<Weekday> {
    Weekday::from_sunday_zero_offset(i8::try_from(number).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn a_wake_reads_a_whole_batch() {
        assert_eq!(usize::try_from(PER_WAKE).unwrap(), BATCH_COUNT);
    }

    #[test]
    fn urgent_events_wake_at_once_and_others_gather() {
        let first = at("2026-11-26T10:00:00Z");
        assert!(wakes_now(&["note", "checkin"], first, first));
        assert!(!wakes_now(&["note"], first, at("2026-11-26T10:01:00Z")));
        assert!(wakes_now(&["note"], first, at("2026-11-26T10:01:30Z")));
        assert!(wakes_now(&["note"; 10], first, first));
        assert!(!wakes_now(&[], first, at("2026-11-27T10:00:00Z")));
    }

    #[test]
    fn at_most_twenty_wakes_an_hour() {
        let start = at("2026-11-26T10:00:00Z");
        let mut budget = WakeBudget::default();
        for n in 0..20 {
            assert!(budget.take(start), "wake {n}");
        }
        assert!(!budget.take(at("2026-11-26T10:59:00Z")));
        assert!(budget.take(at("2026-11-26T11:00:01Z")));
    }

    #[test]
    fn a_wake_is_about_its_kinds() {
        assert_eq!(about("- [checkin] Your check-in: the build"), "Check-in");
        assert_eq!(about("- [note] a\n- [watch] b"), "2 updates");
        assert_eq!(about("- [note] something"), "Update");
        assert_eq!(about("no lines"), "Update");
        assert_eq!(
            event_line("room", "From Main: hi"),
            "- [room] From Main: hi"
        );
    }

    #[test]
    fn quiet_hours_may_run_past_midnight() {
        let night = QuietHours {
            from: hour_minute("23:00").unwrap(),
            to: hour_minute("07:00").unwrap(),
        };
        assert!(night.contains(hour_minute("23:30").unwrap()));
        assert!(night.contains(hour_minute("06:59").unwrap()));
        assert!(!night.contains(hour_minute("07:00").unwrap()));
        let lunch = QuietHours {
            from: hour_minute("12:00").unwrap(),
            to: hour_minute("13:00").unwrap(),
        };
        assert!(lunch.contains(hour_minute("12:30").unwrap()));
        assert!(!lunch.contains(hour_minute("13:30").unwrap()));
    }

    #[test]
    fn times_and_routines_read_as_written() {
        assert_eq!(hour_minute("08:30"), Time::new(8, 30, 0, 0).ok());
        for bad in ["8:30", "24:00", "08:60", "0830", "aa:bb"] {
            assert_eq!(hour_minute(bad), None, "{bad}");
        }
        let half_past_eight = hour_minute("08:30").unwrap();
        let nine = hour_minute("09:00").unwrap();
        assert!(routine_due(half_past_eight, None, nine, Weekday::Monday));
        assert!(!routine_due(nine, None, half_past_eight, Weekday::Monday));
        let thursday = weekday_of(4);
        assert_eq!(thursday, Some(Weekday::Thursday));
        assert!(!routine_due(
            half_past_eight,
            thursday,
            nine,
            Weekday::Monday
        ));
        assert_eq!(weekday_of(7), None);
    }
}
