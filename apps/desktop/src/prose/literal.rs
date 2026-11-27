//! Text that reads as it was typed in a markdown view: every mark that markdown would act on
//! is escaped, line breaks stay breaks, and leading spaces stay spaces (rather than making a
//! code block). For the lines a person typed, which show as they wrote them, selectable, in the
//! views that otherwise draw markdown.

/// `text` as markdown that shows exactly `text`.
#[must_use]
pub(crate) fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        // No-break spaces keep the indent without starting a code block.
        out.extend(std::iter::repeat_n('\u{a0}', indent));
        for c in line[indent..].chars() {
            if c.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(c);
        }
        let next_is_text = lines.get(index + 1).is_some_and(|next| !next.is_empty());
        if index + 1 < lines.len() {
            // A backslash at the end of a line is a hard break; a blank line stays a paragraph's
            // end.
            if !line.is_empty() && next_is_text {
                out.push('\\');
            }
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_escaped_so_they_show() {
        assert_eq!(literal("*hi* _there_"), r"\*hi\* \_there\_");
        assert_eq!(literal("# not a heading"), r"\# not a heading");
        assert_eq!(literal("1. one"), r"1\. one");
        assert_eq!(literal("<b>x</b>"), r"\<b\>x\<\/b\>");
        assert_eq!(literal("a [link](x)"), r"a \[link\]\(x\)");
        assert_eq!(literal("plain words"), "plain words");
        assert_eq!(literal("café – ok"), "café – ok", "only ASCII marks");
    }

    #[test]
    fn lines_stay_lines_and_indents_stay() {
        assert_eq!(literal("one\ntwo"), "one\\\ntwo");
        assert_eq!(literal("one\n\ntwo"), "one\n\ntwo");
        assert_eq!(literal("    code?"), "\u{a0}\u{a0}\u{a0}\u{a0}code\\?");
        assert_eq!(literal(""), "");
    }
}
