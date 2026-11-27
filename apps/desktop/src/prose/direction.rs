//! Which way a text reads: Arabic and Hebrew from the right, everything else from the left.
//!
//! Two rules, as the reference client has them, for two kinds of text:
//!
//! - A message's paragraphs, list items, headings and quotes take the direction of their first
//!   letter ([`of_first_letter`]): an assistant's answer in Arabic starts with an Arabic word.
//! - A card's or a task's short texts take the direction of most of their letters
//!   ([`of_most_letters`]): "ليلى reads the morning paper" is an English sentence with a name in
//!   it, and read from the right it came out scrambled. Such a text is drawn [`marked`], with an
//!   invisible direction mark first, because the text layout sets a line's direction from its
//!   first letter whatever the alignment.
//!
//! [`runs`] cuts a message's markdown into runs of one direction, so the chat lays the
//! right-to-left ones out from the right edge.

use std::sync::Arc;

use super::blocks::{is_heading, is_item};

/// The way a text reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Direction {
    /// From the left edge (English, and anything without a right-to-left letter).
    #[default]
    LeftToRight,
    /// From the right edge (Arabic, Hebrew).
    RightToLeft,
}

impl Direction {
    /// Whether it reads from the right.
    #[must_use]
    pub(crate) const fn is_rtl(self) -> bool {
        matches!(self, Self::RightToLeft)
    }
}

/// U+200F RIGHT-TO-LEFT MARK: invisible, makes a line read from the right.
const RLM: char = '\u{200F}';
/// U+200E LEFT-TO-RIGHT MARK: invisible, makes a line read from the left.
const LRM: char = '\u{200E}';

/// A letter of a right-to-left script: Hebrew, Arabic and their supplements, and the
/// presentation forms.
const fn is_rtl_letter(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Arabic-Indic digits, which have no direction of their own.
const fn is_arabic_digit(c: char) -> bool {
    matches!(c as u32, 0x0660..=0x0669 | 0x06F0..=0x06F9)
}

/// The direction of `text`'s first letter: Arabic or Hebrew reads from the right, a Latin letter
/// from the left; digits, symbols and markdown don't count. No letter reads from the left.
#[must_use]
pub(crate) fn of_first_letter(text: &str) -> Direction {
    for c in text.chars() {
        if is_rtl_letter(c) && !is_arabic_digit(c) {
            return Direction::RightToLeft;
        }
        if c.is_ascii_alphabetic() || matches!(c as u32, 0xC0..=0x24F) {
            return Direction::LeftToRight;
        }
    }
    Direction::LeftToRight
}

/// The direction most of `text`'s letters read in: from the right when more of them are Arabic
/// or Hebrew than of any other script (digits, punctuation and «» don't count).
#[must_use]
pub(crate) fn of_most_letters(text: &str) -> Direction {
    let (mut rtl, mut ltr) = (0_usize, 0_usize);
    for c in text.chars() {
        if is_arabic_digit(c) {
            continue;
        }
        if is_rtl_letter(c) {
            rtl += 1;
        } else if c.is_alphabetic() {
            ltr += 1;
        }
    }
    if rtl > ltr {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    }
}

/// `text` with the direction mark of [`of_most_letters`] first, so a mixed line keeps its order:
/// "#1 الماتور (The Pump)" read without it as "(The Pump) الماتور #1".
#[must_use]
pub(crate) fn marked(text: &str) -> String {
    let mark = if of_most_letters(text).is_rtl() {
        RLM
    } else {
        LRM
    };
    let mut out = String::with_capacity(text.len() + mark.len_utf8());
    out.push(mark);
    out.push_str(text);
    out
}

/// `text` made to read from the right whatever letters it has (a list's `3.` beside Arabic
/// words reads with its full stop on the left, as in Arabic).
#[must_use]
pub(crate) fn right_to_left(text: &str) -> String {
    format!("{RLM}{text}")
}

/// A run of a message's markdown that reads one way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Run {
    /// Markdown that reads from the left, for one markdown view as it is.
    LeftToRight(Arc<str>),
    /// Parts that read from the right, each laid out on its own from the right edge.
    RightToLeft(Vec<Part>),
}

impl Run {
    /// The run with each of its markdown texts made over by `change` (linked, for the chat).
    #[must_use]
    pub(crate) fn map_markdown(self, change: impl Fn(&str) -> String) -> Self {
        match self {
            Self::LeftToRight(text) => Self::LeftToRight(change(&text).into()),
            Self::RightToLeft(parts) => Self::RightToLeft(
                parts
                    .into_iter()
                    .map(|part| Part {
                        markdown: change(&part.markdown).into(),
                        ..part
                    })
                    .collect(),
            ),
        }
    }
}

/// A paragraph, heading or list item that reads from the right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Part {
    /// A list item's marker (`•`, `3.`), drawn at its right; `None` for a paragraph or heading.
    pub(crate) marker: Option<String>,
    /// How deep a list item is nested (0 at the top).
    pub(crate) depth: usize,
    /// Its markdown, without the list marker.
    pub(crate) markdown: Arc<str>,
}

/// A run being read: a left-to-right one still grows.
enum Reading {
    LeftToRight(String),
    RightToLeft(Vec<Part>),
}

/// One paragraph, heading, list item, rule or code block, as lines of the markdown.
struct Piece<'a> {
    lines: Vec<&'a str>,
    direction: Direction,
    item: bool,
}

/// `markdown` (one block of a message) cut into runs of one direction. Markdown that reads from
/// the left throughout comes back as one run, unchanged. Code blocks and rules always read from
/// the left; every other piece takes the direction of its first letter.
#[must_use]
pub(crate) fn runs(markdown: &str) -> Vec<Run> {
    let pieces = pieces(markdown);
    if pieces.iter().all(|piece| !piece.direction.is_rtl()) {
        return vec![Run::LeftToRight(markdown.into())];
    }
    let mut out: Vec<Reading> = Vec::new();
    let mut after_item = false;
    for piece in pieces {
        let item = piece.item;
        match (piece.direction, out.last_mut()) {
            (Direction::LeftToRight, Some(Reading::LeftToRight(text))) => {
                // Items of one list stay one list; anything else keeps its blank line.
                text.push_str(if item && after_item { "\n" } else { "\n\n" });
                text.push_str(&piece.lines.join("\n"));
            }
            (Direction::LeftToRight, _) => {
                out.push(Reading::LeftToRight(piece.lines.join("\n")));
            }
            (Direction::RightToLeft, last) => {
                let part = part(&piece.lines);
                if let Some(Reading::RightToLeft(parts)) = last {
                    parts.push(part);
                } else {
                    out.push(Reading::RightToLeft(vec![part]));
                }
            }
        }
        after_item = item;
    }
    out.into_iter()
        .map(|run| match run {
            Reading::LeftToRight(text) => Run::LeftToRight(text.into()),
            Reading::RightToLeft(parts) => Run::RightToLeft(parts),
        })
        .collect()
}

/// The pieces of `markdown`, in order; blank lines only separate them.
fn pieces(markdown: &str) -> Vec<Piece<'_>> {
    let mut out: Vec<Piece<'_>> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    for line in markdown.split('\n') {
        let trimmed = line.trim();
        let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if let Some(lines) = code.as_mut() {
            lines.push(line);
            if fence {
                let lines = code.take().unwrap_or_default();
                out.push(Piece {
                    lines,
                    direction: Direction::LeftToRight,
                    item: false,
                });
            }
            continue;
        }
        if fence {
            flush_paragraph(&mut paragraph, &mut out);
            code = Some(vec![line]);
        } else if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut out);
        } else if is_item(trimmed) || is_heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(Piece {
                lines: vec![line],
                direction: of_first_letter(trimmed),
                item: is_item(trimmed),
            });
        } else if matches!(trimmed, "---" | "***" | "___") {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(Piece {
                lines: vec![line],
                direction: Direction::LeftToRight,
                item: false,
            });
        } else {
            paragraph.push(line);
        }
    }
    flush_paragraph(&mut paragraph, &mut out);
    if let Some(lines) = code {
        // A code block left open runs to the end, as markdown reads it.
        out.push(Piece {
            lines,
            direction: Direction::LeftToRight,
            item: false,
        });
    }
    out
}

/// The paragraph read so far as a piece, in the direction of its first letter.
fn flush_paragraph<'a>(paragraph: &mut Vec<&'a str>, out: &mut Vec<Piece<'a>>) {
    if !paragraph.is_empty() {
        let lines = std::mem::take(paragraph);
        let direction = of_first_letter(&lines.join(" "));
        out.push(Piece {
            lines,
            direction,
            item: false,
        });
    }
}

/// A right-to-left piece as a part: a list item's marker and depth taken off its words.
fn part(lines: &[&str]) -> Part {
    let first = lines.first().copied().unwrap_or_default();
    let trimmed = first.trim_start();
    if !is_item(trimmed.trim_end()) {
        return Part {
            marker: None,
            depth: 0,
            markdown: lines.join("\n").into(),
        };
    }
    let indent = first.len() - trimmed.len();
    let (marker, words) = split_marker(trimmed);
    Part {
        marker: Some(marker),
        depth: indent / 2,
        markdown: words.trim().into(),
    }
}

/// A list item's marker as drawn (`•` for `-`, `*` and `•`; the number for `3.` or `3)`) and
/// its words.
fn split_marker(item: &str) -> (String, &str) {
    if let Some(rest) = item.strip_prefix(['-', '*', '•']) {
        return ("•".to_owned(), rest);
    }
    let digits = item.len() - item.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let (number, rest) = item.split_at(digits);
    (format!("{number}."), rest.get(1..).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_letter_decides_a_paragraph() {
        assert_eq!(of_first_letter("مرحبا Emberhut"), Direction::RightToLeft);
        assert_eq!(of_first_letter("**3** — שלום"), Direction::RightToLeft);
        assert_eq!(of_first_letter("Emberhut مرحبا"), Direction::LeftToRight);
        assert_eq!(of_first_letter("١٢٣ Hello"), Direction::LeftToRight);
        assert_eq!(of_first_letter("12, 34 — !"), Direction::LeftToRight);
        assert_eq!(of_first_letter(""), Direction::LeftToRight);
    }

    #[test]
    fn most_letters_decide_a_card() {
        assert_eq!(
            of_most_letters("ليلى reads the morning paper"),
            Direction::LeftToRight
        );
        assert_eq!(
            of_most_letters("#1 الماتورات (Pump)"),
            Direction::RightToLeft
        );
        assert_eq!(of_most_letters("«١٢٣» ok"), Direction::LeftToRight);
        assert_eq!(of_most_letters("..."), Direction::LeftToRight);
    }

    #[test]
    fn a_marked_text_starts_with_its_direction_mark() {
        assert_eq!(marked("أهلا"), "\u{200F}أهلا");
        assert_eq!(marked("Hello"), "\u{200E}Hello");
        assert_eq!(marked(""), "\u{200E}");
    }

    #[test]
    fn markdown_from_the_left_stays_one_run_as_it_is() {
        let text = "Hello.\n\n- one\n- two\n\n```\nمرحبا\n```";
        assert_eq!(runs(text), [Run::LeftToRight(text.into())]);
    }

    #[test]
    fn arabic_paragraphs_items_and_headings_read_from_the_right() {
        let text = "## العنوان\n\nفقرة أولى.\n\n- بند أول\n  - بند **ثان**\n3) ثالث\n\nThe end.";
        let rtl = |marker: Option<&str>, depth, markdown: &str| Part {
            marker: marker.map(str::to_owned),
            depth,
            markdown: markdown.into(),
        };
        assert_eq!(
            runs(text),
            [
                Run::RightToLeft(vec![
                    rtl(None, 0, "## العنوان"),
                    rtl(None, 0, "فقرة أولى."),
                    rtl(Some("•"), 0, "بند أول"),
                    rtl(Some("•"), 1, "بند **ثان**"),
                    rtl(Some("3."), 0, "ثالث"),
                ]),
                Run::LeftToRight("The end.".into()),
            ]
        );
    }

    #[test]
    fn code_and_english_between_arabic_keep_their_own_runs() {
        let text = "Intro\n\n- a\n- b\n\nنص\n\n```\nlet x = 1;\n\nlet y = 2;\n```";
        assert_eq!(
            runs(text),
            [
                Run::LeftToRight("Intro\n\n- a\n- b".into()),
                Run::RightToLeft(vec![Part {
                    marker: None,
                    depth: 0,
                    markdown: "نص".into()
                }]),
                Run::LeftToRight("```\nlet x = 1;\n\nlet y = 2;\n```".into()),
            ]
        );
    }
}
