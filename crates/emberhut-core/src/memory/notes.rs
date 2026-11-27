//! Quick notes and the diary: what the owner writes in a few seconds from the app (a thought,
//! what happened, an idea, a to-do), grouped by day into a diary the owner and the assistant
//! share, and the assistant's one short entry a day.
//!
//! The app saves offline and sends each note with its own id (`uid`), so a resend changes
//! nothing; an empty text deletes the note. A note's day is the day in the server's time zone
//! when it was written.

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::cut;

/// The longest note, in characters.
pub const MAX_NOTE: usize = 20_000;

/// The shortest and longest assistant diary entry, in characters (the tool's limits).
pub const ENTRY_LENGTH: (usize, usize) = (10, 2000);

/// How far ahead of the server's clock a note's own time may be before it is taken as wrong.
pub const CLOCK_SLACK_MS: i64 = 60_000;

/// The most new notes put in front of the assistant at once.
pub const NEWS_SHOWN: usize = 20;

/// Who wrote a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoteAuthor {
    /// The owner, from the app.
    Owner,
    /// The assistant: its one entry a day.
    Assistant,
}

/// A note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Its id, the app's (the assistant's entry: [`assistant_uid`] of its day).
    pub uid: String,
    /// Who wrote it.
    pub who: NoteAuthor,
    /// The day it belongs to (`2026-11-24`).
    pub day: String,
    /// When it was written (the device's clock: an offline note keeps its time).
    pub at: Timestamp,
    /// What it says.
    pub text: String,
    /// When the server first had it.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
}

/// Why a note was not taken.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NoteRefusal {
    /// The note's id is not 8 to 64 letters, digits, `_` or `-`.
    #[error("a note needs its uid")]
    Uid,
    /// The note is longer than [`MAX_NOTE`].
    #[error("a quick note is at most 20,000 characters")]
    TooLong,
    /// A diary day that is not `YYYY-MM-DD`, or after today.
    #[error("day: YYYY-MM-DD, today or before")]
    Day,
}

/// The assistant's entry's id for `day`: [`super::wire::DIARY_PREFIX`] and the day.
#[must_use]
pub fn assistant_uid(day: &str) -> String {
    format!("{}{day}", super::wire::DIARY_PREFIX)
}

/// Checks the app's note id.
///
/// # Errors
///
/// [`NoteRefusal::Uid`].
pub fn check_uid(uid: &str) -> Result<(), NoteRefusal> {
    let shaped = (8..=64).contains(&uid.len())
        && uid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if shaped {
        Ok(())
    } else {
        Err(NoteRefusal::Uid)
    }
}

/// The note's text as kept (trimmed), when it is not too long.
///
/// # Errors
///
/// [`NoteRefusal::TooLong`].
pub fn checked_text(text: &str) -> Result<&str, NoteRefusal> {
    let text = text.trim();
    if text.chars().count() > MAX_NOTE {
        Err(NoteRefusal::TooLong)
    } else {
        Ok(text)
    }
}

/// When a note was written: the time its device gave, unless that is missing, unreadable or
/// more than a minute ahead of `now` (a clock in the future isn't when it was written).
#[must_use]
pub fn written_at(given: Option<&str>, now: Timestamp) -> Timestamp {
    given
        .and_then(|given| given.trim().parse::<Timestamp>().ok())
        .filter(|at| at.as_millisecond() <= now.as_millisecond() + CLOCK_SLACK_MS)
        .unwrap_or(now)
}

/// `at`'s day in `zone` (`2026-11-24`).
#[must_use]
pub fn day_of(at: Timestamp, zone: &TimeZone) -> String {
    at.to_zoned_datetime(zone.clone())
        .strftime("%Y-%m-%d")
        .to_string()
}

/// `at`'s time of day in `zone` (`21:57`).
#[must_use]
pub fn hour_minute(at: Timestamp, zone: &TimeZone) -> String {
    at.to_zoned_datetime(zone.clone())
        .strftime("%H:%M")
        .to_string()
}

/// Checks a diary day for the assistant's entry: `YYYY-MM-DD`, `today` or before.
///
/// # Errors
///
/// [`NoteRefusal::Day`].
pub fn check_day(day: &str, today: &str) -> Result<(), NoteRefusal> {
    let shaped = day.len() == 10
        && day.char_indices().all(|(at, c)| {
            if at == 4 || at == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if shaped && day <= today {
        Ok(())
    } else {
        Err(NoteRefusal::Day)
    }
}

/// The notes tool's answer: day by day, newest day first (`notes` comes newest first), the
/// owner's notes in the order they were written and then the assistant's entry.
#[must_use]
pub fn render_days(notes: &[Note], zone: &TimeZone) -> String {
    if notes.is_empty() {
        return "No notes on those days.".to_owned();
    }
    let mut days: Vec<(&str, Vec<&Note>)> = Vec::new();
    for note in notes {
        match days.iter_mut().find(|(day, _)| *day == note.day) {
            Some((_, list)) => list.push(note),
            None => days.push((&note.day, vec![note])),
        }
    }
    days.into_iter()
        .map(|(day, mut list)| {
            list.sort_by_key(|note| (note.who == NoteAuthor::Assistant, note.at));
            let lines: Vec<String> = list
                .iter()
                .map(|note| match note.who {
                    NoteAuthor::Assistant => format!("- Your entry: {}", note.text),
                    NoteAuthor::Owner => {
                        format!("- {}: {}", hour_minute(note.at, zone), note.text)
                    }
                })
                .collect();
            format!("{}\n{}", day_heading(day), lines.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `2026-11-24` as `Tuesday 24 Nov 2026`; a day that doesn't read as one stays as it is.
fn day_heading(day: &str) -> String {
    day.parse::<jiff::civil::Date>().map_or_else(
        |_| day.to_owned(),
        |date| date.strftime("%A %-d %b %Y").to_string(),
    )
}

/// The owner's notes new or edited since the assistant last took them (`fresh`, oldest first;
/// `seen`: when it last took them), as a paragraph to add to what wakes it; empty when there
/// are none. The notes are the owner's alone: the paragraph says so.
#[must_use]
pub fn news(fresh: &[Note], seen: Option<Timestamp>, today: &str, zone: &TimeZone) -> String {
    if fresh.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = fresh
        .iter()
        .take(NEWS_SHOWN)
        .map(|note| {
            let day = if note.day == today {
                String::new()
            } else {
                format!("{} ", note.day)
            };
            let edited = if seen.is_some_and(|seen| note.created <= seen) {
                " (edited)"
            } else {
                ""
            };
            format!(
                "- {day}{}{edited}: {}",
                hour_minute(note.at, zone),
                preview(&note.text, 600)
            )
        })
        .collect();
    let more = if fresh.len() > NEWS_SHOWN {
        format!("\n- ...and {} more (notes tool)", fresh.len() - NEWS_SHOWN)
    } else {
        String::new()
    };
    format!(
        "\n\n(New quick notes since you last looked, from the user's diary. Act on a to-do, a \
         promise or something they want done; no need to answer each one. Theirs alone: never \
         quote them to anyone else.\n{}{more})",
        shown.join("\n")
    )
}

/// `text` on one line, cut to `limit` characters with an ellipsis.
fn preview(text: &str, limit: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > limit {
        format!("{}…", cut(&one_line, limit))
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn note(uid: &str, who: NoteAuthor, day: &str, when: &str, text: &str) -> Note {
        Note {
            uid: uid.to_owned(),
            who,
            day: day.to_owned(),
            at: at(when),
            text: text.to_owned(),
            created: at(when),
            updated: at(when),
        }
    }

    #[test]
    fn a_uid_is_the_apps_shape() {
        assert!(check_uid("note-1234").is_ok());
        assert_eq!(check_uid("short"), Err(NoteRefusal::Uid));
        assert_eq!(check_uid("has space here"), Err(NoteRefusal::Uid));
        assert_eq!(check_uid(&"a".repeat(65)), Err(NoteRefusal::Uid));
    }

    #[test]
    fn a_clock_in_the_future_is_not_when_it_was_written() {
        let now = at("2026-11-24T12:00:00Z");
        assert_eq!(
            written_at(Some("2026-11-24T09:00:00Z"), now),
            at("2026-11-24T09:00:00Z")
        );
        assert_eq!(
            written_at(Some("2026-11-24T12:00:30Z"), now),
            at("2026-11-24T12:00:30Z")
        );
        assert_eq!(written_at(Some("2026-11-24T12:05:00Z"), now), now);
        assert_eq!(written_at(Some("soon"), now), now);
        assert_eq!(written_at(None, now), now);
        assert_eq!(
            written_at(None, now + SignedDuration::from_secs(1)),
            now + SignedDuration::from_secs(1)
        );
    }

    #[test]
    fn days_are_the_zones() {
        let zone = TimeZone::fixed(jiff::tz::offset(3));
        assert_eq!(day_of(at("2026-11-24T22:30:00Z"), &zone), "2026-11-25");
        assert_eq!(hour_minute(at("2026-11-24T22:30:00Z"), &zone), "01:30");
    }

    #[test]
    fn a_diary_day_is_today_or_before() {
        assert!(check_day("2026-11-24", "2026-11-24").is_ok());
        assert!(check_day("2026-11-01", "2026-11-24").is_ok());
        assert_eq!(check_day("2026-11-25", "2026-11-24"), Err(NoteRefusal::Day));
        assert_eq!(check_day("24 Nov", "2026-11-24"), Err(NoteRefusal::Day));
    }

    #[test]
    fn the_diary_reads_day_by_day() {
        let notes = [
            note(
                "note-0003",
                NoteAuthor::Owner,
                "2026-11-24",
                "2026-11-24T18:00:00Z",
                "Dinner",
            ),
            note(
                &assistant_uid("2026-11-24"),
                NoteAuthor::Assistant,
                "2026-11-24",
                "2026-11-24T19:00:00Z",
                "A calm day.",
            ),
            note(
                "note-0002",
                NoteAuthor::Owner,
                "2026-11-24",
                "2026-11-24T08:00:00Z",
                "Slept well",
            ),
            note(
                "note-0001",
                NoteAuthor::Owner,
                "2026-11-23",
                "2026-11-23T08:00:00Z",
                "Rain",
            ),
        ];
        assert_eq!(
            render_days(&notes, &TimeZone::UTC),
            "Tuesday 24 Nov 2026\n- 08:00: Slept well\n- 18:00: Dinner\n- Your entry: A calm day.\n\n\
             Monday 23 Nov 2026\n- 08:00: Rain"
        );
        assert_eq!(render_days(&[], &TimeZone::UTC), "No notes on those days.");
    }

    #[test]
    fn news_marks_edits_and_older_days() {
        let mut edited = note(
            "note-0001",
            NoteAuthor::Owner,
            "2026-11-23",
            "2026-11-23T08:00:00Z",
            "Rain",
        );
        edited.updated = at("2026-11-24T10:00:00Z");
        let fresh = note(
            "note-0002",
            NoteAuthor::Owner,
            "2026-11-24",
            "2026-11-24T09:00:00Z",
            "Call  the\nbank",
        );
        let text = news(
            &[edited, fresh],
            Some(at("2026-11-24T00:00:00Z")),
            "2026-11-24",
            &TimeZone::UTC,
        );
        assert!(
            text.contains("- 2026-11-23 08:00 (edited): Rain\n- 09:00: Call the bank)"),
            "{text}"
        );
        assert_eq!(news(&[], None, "2026-11-24", &TimeZone::UTC), "");
    }
}
