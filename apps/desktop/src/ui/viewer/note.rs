//! A note or a text file in the viewer: markdown drawn formatted, any other text as it is (in a
//! fenced block, so nothing in it reads as markdown), selectable, its size following the zoom.
//! A very long file shows its first part, with a line saying so.

use crate::store::files::kind::is_markdown;

/// The most text the viewer lays out (more would make it crawl): the first megabyte.
const MOST: usize = 1 << 20;

/// What the viewer shows of the text file called `name`, as markdown.
#[must_use]
pub(crate) fn markdown_for(name: &str, text: &str) -> String {
    let (shown, cut) = match text.char_indices().nth(MOST) {
        Some((at, _)) => (&text[..at], true),
        None => (text, false),
    };
    let mut out = if is_markdown(name) {
        shown.to_owned()
    } else {
        fenced(shown)
    };
    if cut {
        out.push_str("\n\n*The rest of this file is longer than shows here: open it in its app to read it all.*\n");
    }
    out
}

/// `text` in a fenced block whose fence no run of backticks in it can close.
fn fenced(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{text}\n{fence}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_stays_markdown_and_other_text_is_fenced() {
        assert_eq!(markdown_for("plan.md", "# Plan\n- ship"), "# Plan\n- ship");
        assert_eq!(markdown_for("log.txt", "a *b*"), "```text\na *b*\n```\n");
    }

    #[test]
    fn a_fence_inside_the_text_cant_close_its_block() {
        let shown = markdown_for("notes.txt", "before\n````\nafter");
        assert!(shown.starts_with("`````text\n"), "{shown}");
        assert!(shown.ends_with("\n`````\n"), "{shown}");
    }

    #[test]
    fn a_very_long_file_shows_its_first_part() {
        let long = "x".repeat(MOST + 10);
        let shown = markdown_for("big.md", &long);
        assert!(shown.starts_with(&"x".repeat(MOST)));
        assert!(!shown.contains(&"x".repeat(MOST + 1)));
        assert!(shown.contains("open it in its app"));
    }
}
