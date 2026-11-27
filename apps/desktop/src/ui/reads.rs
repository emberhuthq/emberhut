//! Short texts (a card's title, a task's title) laid out the way most of their letters read:
//! mostly Arabic from the right edge, with a direction mark first so a mixed line keeps its
//! order (see [`crate::prose::direction`]). Longer texts that aren't a chat message (a note, a
//! session's answer, a party's turn) start from the right edge when their first letter does.

use gpui_kit::prelude::*;
use gpui_kit::{Div, Styled, div};

use crate::prose::direction;

/// `text` in a block of the width it is given, from the right edge when most of its letters are
/// Arabic or Hebrew. Style it like any `div` (its size, weight and colour).
pub(crate) fn reads(text: &str) -> Div {
    div()
        .w_full()
        .when(mostly_right(text), Styled::text_right)
        .child(direction::marked(text))
}

/// Whether a short text (a title, a name, a line) sits at the right edge: most of its letters
/// are Arabic or Hebrew. Draw it [`direction::marked`] too, so it keeps its order.
#[must_use]
pub(crate) fn mostly_right(text: &str) -> bool {
    direction::of_most_letters(text).is_rtl()
}

/// Whether a longer text (a note, an answer, a turn) starts at the right edge: its first letter
/// is Arabic or Hebrew, as a chat message's paragraph does.
#[must_use]
pub(crate) fn starts_right(text: &str) -> bool {
    direction::of_first_letter(text).is_rtl()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_follows_most_of_its_letters() {
        assert!(mostly_right("مراجعة الإعلانات"));
        assert!(!mostly_right("ليلى reads the morning paper"));
        assert!(!mostly_right("Site launch"));
        assert!(!mostly_right(""));
    }

    #[test]
    fn a_longer_text_follows_its_first_letter() {
        assert!(starts_right("مرحبا, this is mixed"));
        assert!(!starts_right("Hello مرحبا بك في البيت"));
        assert!(starts_right("1. خطوة أولى"));
        assert!(!starts_right("12 34"));
    }
}
