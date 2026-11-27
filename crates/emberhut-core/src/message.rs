//! Messages: what is said in a room, by whom, and how a room's history is paged.

use jiff::Timestamp;

use crate::ids::{MessageId, RoomId};

/// Messages in one page of a room's history.
pub const PAGE_SIZE: u64 = 60;

/// Who a message is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Author {
    /// The person using the app.
    User,
    /// The assistant's reply.
    Assistant,
    /// A note from the server itself (an event), not part of the conversation.
    Event,
}

/// A message as it was stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Its id. Ids grow, so they order a room's messages.
    pub id: MessageId,
    /// The room it was said in.
    pub room: RoomId,
    /// Who said it.
    pub author: Author,
    /// What was said.
    pub text: String,
    /// When it was said.
    pub created_at: Timestamp,
    /// The message it replies to, if any.
    pub reply_to: Option<MessageId>,
}

/// A message to store; its id and time are given when it is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMessage {
    /// The room it is said in.
    pub room: RoomId,
    /// Who says it.
    pub author: Author,
    /// What is said.
    pub text: String,
    /// The message it replies to, if any.
    pub reply_to: Option<MessageId>,
}

/// Which page of a room's history to read: the newest [`PAGE_SIZE`] messages between two
/// messages (both left out), oldest first.
///
/// With neither bound it is the room's latest page. `before` pages back through older
/// messages; `after` asks for what came since a message the app already has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Only messages older than this one.
    pub before: Option<MessageId>,
    /// Only messages newer than this one.
    pub after: Option<MessageId>,
}

impl HistoryQuery {
    /// The page a request's `?before=` and `?after=` ask for, read leniently: each is a number
    /// (blank is 0; text that isn't a number is none). `before` 0, none or infinite means no
    /// bound; a fraction bounds at the next whole id, and a negative one leaves nothing older.
    /// `after` bounds only when positive and finite.
    ///
    /// ```
    /// use emberhut_core::MessageId;
    /// use emberhut_core::message::HistoryQuery;
    ///
    /// let query = HistoryQuery::lenient(Some("40"), Some("abc"));
    /// assert_eq!(query.before, Some(MessageId::new(40)));
    /// assert_eq!(query.after, None);
    /// assert_eq!(HistoryQuery::lenient(Some("0"), None), HistoryQuery::default());
    /// ```
    #[must_use]
    pub fn lenient(before: Option<&str>, after: Option<&str>) -> Self {
        let before = before
            .map(lenient_number)
            .filter(|n| n.abs() > 0.0 && *n < f64::INFINITY)
            .map(|n| MessageId::new(whole(n.ceil())));
        let after = after
            .map(lenient_number)
            .filter(|n| n.is_finite() && *n > 0.0)
            .map(|n| MessageId::new(whole(n.floor())));
        Self { before, after }
    }
}

/// `text` read as JavaScript's `Number` reads it, near enough: blank is 0, anything that isn't a
/// number is NaN.
fn lenient_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// A whole `f64` as an id, saturating at the ends of `i64`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a whole number; `as` saturates past the ends of i64, which bounds the same"
)]
fn whole(n: f64) -> i64 {
    n as i64
}

/// How long ago `at` was, as the chat shows it next to a message: `now`, `4m`, `3h`,
/// `3h 20m`, `2d`. Minutes are rounded to the nearest; a time in the future reads `now`.
///
/// ```
/// use jiff::{SignedDuration, Timestamp};
/// use emberhut_core::message::age_label;
///
/// let now = Timestamp::from_second(1_800_000_000).unwrap();
/// assert_eq!(age_label(now - SignedDuration::from_secs(20), now), "now");
/// assert_eq!(age_label(now - SignedDuration::from_mins(200), now), "3h 20m");
/// ```
#[must_use]
pub fn age_label(at: Timestamp, now: Timestamp) -> String {
    const MINUTE_MS: i128 = 60_000;
    let elapsed = (now - at).as_millis().max(0);
    let minutes = i64::try_from((elapsed + MINUTE_MS / 2) / MINUTE_MS).unwrap_or(i64::MAX);
    match minutes {
        0 => "now".to_owned(),
        1..60 => format!("{minutes}m"),
        60..1_440 if minutes % 60 == 0 => format!("{}h", minutes / 60),
        60..1_440 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d", minutes / 1_440),
    }
}

/// `text` cut to at most `limit` characters for a one-line preview: a longer text loses the
/// word the cut falls in, and ends with `…`.
///
/// ```
/// use emberhut_core::message::preview;
///
/// assert_eq!(preview("short", 10), "short");
/// assert_eq!(preview("one two three", 9), "one two…");
/// ```
#[must_use]
pub fn preview(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit).collect();
    let kept = match cut.rfind(char::is_whitespace) {
        Some(space) => cut[..space].trim_end(),
        None => &cut,
    };
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    fn ago(minutes: i64, seconds: i64) -> String {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let at = now - SignedDuration::from_mins(minutes) - SignedDuration::from_secs(seconds);
        age_label(at, now)
    }

    #[test]
    fn ages_read_like_the_chat_shows_them() {
        assert_eq!(ago(0, 29), "now");
        assert_eq!(ago(0, 30), "1m", "minutes are rounded");
        assert_eq!(ago(59, 0), "59m");
        assert_eq!(ago(60, 0), "1h");
        assert_eq!(ago(61, 0), "1h 1m");
        assert_eq!(ago(1_439, 0), "23h 59m");
        assert_eq!(ago(1_440, 0), "1d");
        assert_eq!(ago(3 * 1_440 + 5, 0), "3d");
        assert_eq!(ago(-5, 0), "now", "a time ahead of the clock");
    }

    #[test]
    fn history_bounds_are_read_leniently() {
        let query = |before, after| HistoryQuery::lenient(before, after);
        let id = |n| Some(MessageId::new(n));
        assert_eq!(query(None, None), HistoryQuery::default());
        assert_eq!(query(Some(" 12 "), Some("3")).before, id(12));
        assert_eq!(query(Some(" 12 "), Some("3")).after, id(3));
        assert_eq!(query(Some(""), Some("")), HistoryQuery::default());
        assert_eq!(query(Some("abc"), Some("-4")), HistoryQuery::default());
        assert_eq!(query(Some("inf"), Some("inf")), HistoryQuery::default());
        assert_eq!(query(Some("5.5"), None).before, id(6), "id < 5.5 is id < 6");
        assert_eq!(query(None, Some("5.5")).after, id(5), "id > 5.5 is id > 5");
        assert_eq!(query(Some("-3"), None).before, id(-3), "nothing is older");
        assert_eq!(query(Some("-inf"), None).before, id(i64::MIN));
        assert_eq!(query(Some("1e2"), None).before, id(100));
    }

    #[test]
    fn previews_cut_at_a_word() {
        assert_eq!(preview("abc", 3), "abc");
        assert_eq!(
            preview("abcdef", 3),
            "abc…",
            "one long word is cut where it is"
        );
        assert_eq!(preview("é é é é", 4), "é é…");
    }
}
