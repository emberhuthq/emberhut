//! The morning brief and the evening wrap: the daily routines the assistant has until the
//! owner sets their own (`set_routines`), and what a brief's wake carries.
//!
//! A routine is a time of day (and, for a weekly one, a weekday) with what to do; at its time
//! the assistant is woken in Main with that text, once that day. The two default routines are
//! the brief in the morning and the wrap in the evening; the owner's own list replaces them.

/// A daily (or weekly) routine as the defaults give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultRoutine {
    /// Its time of day, `HH:MM` (server time).
    pub at: &'static str,
    /// What to do then.
    pub what: &'static str,
}

/// The morning brief's text: it starts with the words [`is_morning_brief`] looks for.
pub const MORNING_BRIEF: &str = "Morning brief: in a few short lines for the owner's phone, what \
happened overnight (threads, background work, check-ins and watches), who is waiting on them \
and what is due today, and anything worth their attention. Lead with today: the one to three \
things that matter most, prepared. Leave paused projects out unless their date came.";

/// The evening wrap's text.
pub const EVENING_WRAP: &str = "Evening wrap: what got done today and the open loops for \
tomorrow, short and kind. Line up what can run overnight. If nothing is worth saying, reply \
NO_REPLY.";

/// The routines until the owner sets their own.
pub const DEFAULT_ROUTINES: &[DefaultRoutine] = &[
    DefaultRoutine {
        at: "08:30",
        what: MORNING_BRIEF,
    },
    DefaultRoutine {
        at: "21:30",
        what: EVENING_WRAP,
    },
];

/// Whether a routine's text is a morning brief (it starts with "Morning brief", in any case).
#[must_use]
pub fn is_morning_brief(what: &str) -> bool {
    starts_with_words(what, "morning brief")
}

/// Whether a routine's text is a brief or a wrap: the ones that review the day's work.
#[must_use]
pub fn is_review(what: &str) -> bool {
    is_morning_brief(what) || starts_with_words(what, "evening wrap")
}

fn starts_with_words(text: &str, words: &str) -> bool {
    text.trim_start()
        .get(..words.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(words))
}

/// The key that makes a routine's wake happen once on `day` (`2026-11-26`) at `at`.
#[must_use]
pub fn routine_key(day: &str, at: &str) -> String {
    format!("routine:{day}:{at}")
}

/// A brief's wake: the routine's text, then what the rooms said since the last one, so the
/// brief covers every room without asking for each.
#[must_use]
pub fn brief_text(what: &str, rooms_since: &str) -> String {
    if rooms_since.trim().is_empty() {
        what.to_owned()
    } else {
        format!("{what}\nWhat was said in the rooms since yesterday:\n{rooms_since}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_a_brief_and_a_wrap() {
        assert_eq!(DEFAULT_ROUTINES.len(), 2);
        assert!(is_morning_brief(DEFAULT_ROUTINES[0].what));
        assert!(!is_morning_brief(DEFAULT_ROUTINES[1].what));
        assert!(is_review(DEFAULT_ROUTINES[1].what));
        assert!(is_morning_brief("  MORNING BRIEF, short"));
        assert!(!is_review("Weekly meeting"));
        assert!(!is_review("Morning"));
    }

    #[test]
    fn a_brief_carries_what_the_rooms_said() {
        assert_eq!(
            routine_key("2026-11-26", "08:30"),
            "routine:2026-11-26:08:30"
        );
        assert_eq!(brief_text("Morning brief.", " "), "Morning brief.");
        assert_eq!(
            brief_text("Morning brief.", "## Main\nhello"),
            "Morning brief.\nWhat was said in the rooms since yesterday:\n## Main\nhello"
        );
    }
}
