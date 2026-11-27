//! How a room's row in the sidebar reads: its last line (markdown taken out, "You: " when it
//! was the user's), when it last spoke, and the badge at its end. Pure functions of the row and
//! "now", so they are tested without a clock.

use emberhut_api::rooms::RoomRow;
use jiff::{Timestamp, ZonedDateTime};

use crate::store::lookup;

/// Below this width the sidebar draws one line per room: no last line, the needs count or
/// Paused as the row's badge.
pub(crate) const WIDE_FROM: f32 = 236.;

/// What a room's row shows besides its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomLine {
    /// The last line said there, cleaned; none while paused or archived, or in a narrow
    /// sidebar.
    pub(crate) preview: Option<String>,
    /// When it last spoke (`now`, `2m`, `3h`, `Fri`, `Oct 2`), shown beside the name with the
    /// preview.
    pub(crate) ago: Option<String>,
    /// The badge at the row's end when there is no preview: `Back Oct 2` or `Paused` for a
    /// paused room, else how many need the user.
    pub(crate) badge: Option<String>,
    /// How many need the user, at the end of the preview line.
    pub(crate) needs: Option<u32>,
    /// Whether it has answers the user hasn't seen (never while paused or archived).
    pub(crate) unread: bool,
}

impl RoomLine {
    /// The row of `room` at `now`, in a sidebar `wide` enough for two lines.
    #[must_use]
    pub(crate) fn of(room: &RoomRow, wide: bool, now: &ZonedDateTime) -> Self {
        let quiet = room.is_paused() || room.is_archived();
        let preview = (wide && !quiet).then(|| preview(room)).flatten();
        let needs = (room.needs > 0).then_some(room.needs);
        let badge = if preview.is_some() {
            None
        } else if room.is_paused() {
            Some(paused_badge(room))
        } else {
            needs.map(|needs| needs.to_string())
        };
        Self {
            ago: preview
                .is_some()
                .then(|| {
                    ago(
                        room.last_at
                            .as_ref()
                            .map(emberhut_api::time::Timestamp::as_str),
                        now,
                    )
                })
                .flatten(),
            needs: needs.filter(|_| preview.is_some()),
            preview,
            badge,
            unread: room.unread > 0 && !quiet,
        }
    }
}

/// A paused room's badge: `Back Oct 2` when it has a day to come back, else `Paused`.
#[must_use]
pub(crate) fn paused_badge(room: &RoomRow) -> String {
    room.paused_until
        .as_ref()
        .and_then(|day| lookup::civil_day(day.as_str()))
        .map_or_else(
            || "Paused".to_owned(),
            |day| format!("Back {}", day.strftime("%b %-d")),
        )
}

/// A room's last line as the sidebar shows it: markdown taken out, a leading heading, list or
/// quote mark dropped, `You: ` before it when the user said it; none when nothing is left.
#[must_use]
pub(crate) fn preview(room: &RoomRow) -> Option<String> {
    let last = room.last.as_deref()?;
    let text = plain(last);
    let text = drop_leading_mark(text.trim()).trim();
    if text.is_empty() {
        return None;
    }
    let you = if room.last_mine { "You: " } else { "" };
    Some(format!("{you}{text}"))
}

/// `text` without inline markdown: emphasis, strike and code marks that pair up go (a lone
/// one, as in `2*3`, stays), a link keeps its words, and line breaks become spaces.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        match c {
            '*' | '`' | '~' => {
                let run = &rest[..rest.len() - rest.trim_start_matches(c).len()];
                match paired(run, &rest[run.len()..]) {
                    Some((inside, after)) if c == '`' => {
                        out.push_str(inside);
                        rest = after;
                    }
                    Some((inside, after)) => {
                        out.push_str(&plain(inside));
                        rest = after;
                    }
                    None => {
                        out.push_str(run);
                        rest = &rest[run.len()..];
                    }
                }
            }
            // `_` marks emphasis only at a word's edge: snake_case keeps its underscores.
            '_' if is_edge(&out, &rest[1..]) => rest = &rest[1..],
            '!' if rest[1..].starts_with('[') => rest = &rest[1..],
            '[' => {
                if let Some((words, after)) = link(rest) {
                    out.push_str(words);
                    rest = after;
                } else {
                    out.push('[');
                    rest = &rest[1..];
                }
            }
            '\n' | '\r' | '\t' => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                rest = &rest[1..];
            }
            c => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

/// The text a `run` of marks (`*`, `**`, `` ` ``, `~~`) opens in `after`, up to the same run
/// closing it, and what follows; none when nothing closes it or it wraps nothing (or spaces
/// at its edges, which markdown doesn't take for emphasis).
fn paired<'a>(run: &str, after: &'a str) -> Option<(&'a str, &'a str)> {
    let end = after.find(run)?;
    let inside = &after[..end];
    let edges = |s: &str| s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace);
    if inside.is_empty() || edges(inside) {
        return None;
    }
    Some((inside, &after[end + run.len()..]))
}

/// Whether an `_` between `before` and `after` sits at a word's edge.
fn is_edge(before: &str, after: &str) -> bool {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    !(word(before.chars().last()) && word(after.chars().next()))
}

/// A link `[words](target)` at the start of `text`: its words and what follows it.
fn link(text: &str) -> Option<(&str, &str)> {
    let close = text.find("](")?;
    let words = &text[1..close];
    if words.contains('[') {
        return None;
    }
    let target = &text[close + 2..];
    let end = target.find(')')?;
    Some((words, &target[end + 1..]))
}

/// `text` without a leading heading (`#`…`######`), list (`-`, `*`, `1.`) or quote (`>`) mark.
fn drop_leading_mark(text: &str) -> &str {
    let (mark, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let heading = !mark.is_empty() && mark.len() <= 6 && mark.chars().all(|c| c == '#');
    let bullet = matches!(mark, "-" | "*" | "+" | ">");
    let numbered = mark
        .strip_suffix('.')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if heading || bullet || numbered {
        rest.trim_start()
    } else {
        text
    }
}

/// When a room last spoke, short: `now`, `2m`, `3h`, the weekday (`Fri`) within six days, then
/// the day (`Oct 2`), in `now`'s time zone.
#[must_use]
pub(crate) fn ago(at: Option<&str>, now: &ZonedDateTime) -> Option<String> {
    let at: Timestamp = at?.parse().ok()?;
    let seconds = (now.timestamp().as_second() - at.as_second()).max(0);
    Some(match seconds {
        0..60 => "now".to_owned(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => {
            let day = at.to_zoned_datetime(now.time_zone().clone());
            if seconds < 6 * 86_400 {
                day.strftime("%a").to_string()
            } else {
                day.strftime("%b %-d").to_string()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::fixtures::room;

    fn now() -> ZonedDateTime {
        "2026-01-10T12:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC)
    }

    fn said(last: &str, mine: bool) -> RoomRow {
        room(2, json!({"last": last, "lastMine": mine}))
    }

    #[test]
    fn the_last_line_loses_its_markdown() {
        let cases = [
            ("**Done**: the `build` passes", "Done: the build passes"),
            (
                "See [the report](/reports/a.html) now",
                "See the report now",
            ),
            ("## Plan for today", "Plan for today"),
            ("- first thing", "first thing"),
            ("> a quote", "a quote"),
            ("12. twelfth", "twelfth"),
            ("_soft_ and snake_case", "soft and snake_case"),
            ("two\nlines", "two lines"),
            ("#hashtag stays", "#hashtag stays"),
            ("a [bracket", "a [bracket"),
            ("2*3 is 6, ~5 left", "2*3 is 6, ~5 left"),
            ("* a *b* and ~~old~~ c", "a b and old c"),
            ("`x*y*z`", "x*y*z"),
        ];
        for (last, shown) in cases {
            assert_eq!(
                preview(&said(last, false)).as_deref(),
                Some(shown),
                "{last}"
            );
        }
    }

    #[test]
    fn the_users_own_line_says_you() {
        assert_eq!(
            preview(&said("on my way", true)).as_deref(),
            Some("You: on my way")
        );
        assert_eq!(preview(&said("  ", true)), None, "nothing left to show");
        assert_eq!(preview(&said("- ", false)), None);
    }

    #[test]
    fn when_it_last_spoke_reads_short() {
        let ago = |at: &str| ago(Some(at), &now()).unwrap();
        assert_eq!(ago("2026-01-10T11:59:30Z"), "now");
        assert_eq!(ago("2026-01-10T11:58:00Z"), "2m");
        assert_eq!(ago("2026-01-10T09:00:00Z"), "3h");
        assert_eq!(ago("2026-01-09T08:00:00Z"), "Fri");
        assert_eq!(ago("2026-01-05T12:00:00Z"), "Mon");
        assert_eq!(ago("2025-10-02T12:00:00Z"), "Oct 2");
        assert_eq!(super::ago(None, &now()), None);
    }

    #[test]
    fn a_wide_row_has_its_last_line_and_needs_at_its_end() {
        let row = room(
            2,
            json!({"last": "hi", "lastAt": "2026-01-10T11:58:00Z", "needs": 2, "unread": 1}),
        );
        let line = RoomLine::of(&row, true, &now());
        assert_eq!(line.preview.as_deref(), Some("hi"));
        assert_eq!(line.ago.as_deref(), Some("2m"));
        assert_eq!(line.needs, Some(2));
        assert_eq!(line.badge, None);
        assert!(line.unread);
    }

    #[test]
    fn a_narrow_row_or_one_with_no_last_line_badges_its_needs() {
        let row = room(2, json!({"last": "hi", "needs": 3}));
        let narrow = RoomLine::of(&row, false, &now());
        assert_eq!(narrow.preview, None);
        assert_eq!(narrow.badge.as_deref(), Some("3"));
        let silent = RoomLine::of(&room(2, json!({"needs": 1})), true, &now());
        assert_eq!(silent.badge.as_deref(), Some("1"));
    }

    #[test]
    fn a_paused_room_says_when_it_is_back_and_nothing_else() {
        let row = room(
            3,
            json!({"active": false, "status": "paused", "pausedUntil": "2026-10-02",
                   "last": "hi", "unread": 4, "needs": 2}),
        );
        let line = RoomLine::of(&row, true, &now());
        assert_eq!(line.preview, None);
        assert_eq!(line.badge.as_deref(), Some("Back Oct 2"));
        assert!(!line.unread);
        let open_ended = room(3, json!({"active": false, "status": "paused"}));
        assert_eq!(paused_badge(&open_ended), "Paused");
    }
}
