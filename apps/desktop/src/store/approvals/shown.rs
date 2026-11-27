//! An approval request's words as the user is shown them: exactly as they are, but with every
//! character that changes how text reads without showing itself written out as `[U+XXXX]`, so
//! a target or a text can't show one thing and say another. Those are the format characters
//! (Unicode's `Cf`: bidirectional overrides and isolates, zero-width characters, the byte-order
//! mark…), by the server's own rule for its link page and Slack message, and the other control
//! characters but a line break and a tab, with the line and paragraph separators.

use std::borrow::Cow;
use std::fmt::Write as _;

use emberhut_core::approvals::shown::is_invisible;

/// Whether `c` is written out rather than shown.
#[must_use]
fn is_hidden(c: char) -> bool {
    is_invisible(c)
        || (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{2028}' | '\u{2029}')
}

/// `text` with every hidden character written out as `[U+XXXX]`; the rest as it is.
#[must_use]
pub(crate) fn visible(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_hidden) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        if is_hidden(c) {
            // Writing to a String never fails.
            let _written = write!(out, "[U+{:04X}]", u32::from(c));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_characters_are_written_out() {
        assert_eq!(
            visible("example.com/\u{202E}gol"),
            "example.com/[U+202E]gol"
        );
        assert_eq!(
            visible("a\u{2066}b\u{2069}c\u{200E}\u{200F}"),
            "a[U+2066]b[U+2069]c[U+200E][U+200F]"
        );
        assert_eq!(
            visible("zero\u{200B}\u{200C}\u{200D}\u{2060}\u{FEFF}width"),
            "zero[U+200B][U+200C][U+200D][U+2060][U+FEFF]width"
        );
        assert_eq!(visible("tag\u{E0041}"), "tag[U+E0041]");
    }

    #[test]
    fn controls_but_a_line_break_and_a_tab_are_written_out() {
        assert_eq!(visible("one\ntwo\tthree"), "one\ntwo\tthree");
        assert_eq!(visible("bell\u{7}\r\n"), "bell[U+0007][U+000D]\n");
        assert_eq!(visible("a\u{85}b\u{2028}c"), "a[U+0085]b[U+2028]c");
    }

    #[test]
    fn the_rest_stays_exactly_as_it_is() {
        let text = "  Two spaces, é, مرحبا, 日本, 👍🏽 and a soft\u{00A0}space.";
        assert!(matches!(visible(text), Cow::Borrowed(same) if same == text));
    }
}
