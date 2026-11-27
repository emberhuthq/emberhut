//! How elements, menus and times are written in the answers, as the reference helper writes
//! them.

/// `text` on one line (a new line reads ` ⏎ `), cut to `max` characters with `…`.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    let one = text.replace('\n', " ⏎ ");
    if one.chars().count() > max {
        let mut cut: String = one.chars().take(max).collect();
        cut.push('…');
        cut
    } else {
        one
    }
}

/// An accessibility role as words: `AXButton` is "button", `AXTextField` "text field".
#[must_use]
pub fn kind(role: &str) -> String {
    let bare = role.replace("AX", "");
    let mut words = String::with_capacity(bare.len() + 4);
    let mut previous_lower = false;
    for c in bare.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            words.push(' ');
        }
        previous_lower = c.is_ascii_lowercase();
        words.push(c.to_ascii_lowercase());
    }
    words
}

/// A menu title as it is matched: lower case, without `…` or `...`, trimmed.
#[must_use]
pub fn menu_key(title: &str) -> String {
    title
        .to_lowercase()
        .replace('…', "")
        .replace("...", "")
        .trim()
        .to_owned()
}

/// A length of time as a clock: `4:05`, or `1:02:03` past an hour.
#[must_use]
pub fn clock(seconds: f64) -> String {
    // Whole seconds of a recording: never negative, never near u64's end.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a recording's whole seconds, clamped to zero and up"
    )]
    let n = seconds.max(0.0).floor() as u64;
    if n >= 3600 {
        format!("{}:{:02}:{:02}", n / 3600, n % 3600 / 60, n % 60)
    } else {
        format!("{}:{:02}", n / 60, n % 60)
    }
}

/// Appends `part` to `line` (writing to a `String` never fails).
pub fn push(line: &mut String, part: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    let _written = line.write_fmt(part);
}

/// A list as the answers write one: `1. Main, 2. untitled`.
#[must_use]
pub fn numbered(titles: &[String]) -> String {
    titles
        .iter()
        .enumerate()
        .map(|(i, title)| format!("{}. {title}", i + 1))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_keeps_one_line_and_cuts_long_text() {
        assert_eq!(clip("a\nb", 10), "a ⏎ b");
        assert_eq!(clip("abcdef", 3), "abc…");
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("ééééé", 2), "éé…");
    }

    #[test]
    fn roles_read_as_words() {
        assert_eq!(kind("AXButton"), "button");
        assert_eq!(kind("AXTextField"), "text field");
        assert_eq!(kind("AXMenuBarItem"), "menu bar item");
        assert_eq!(kind("AXSecureTextField"), "secure text field");
        assert_eq!(kind(""), "");
    }

    #[test]
    fn menu_titles_match_without_ellipses_or_case() {
        assert_eq!(menu_key(" Export… "), "export");
        assert_eq!(menu_key("Save As..."), "save as");
    }

    #[test]
    fn clocks_count_minutes_then_hours() {
        assert_eq!(clock(0.4), "0:00");
        assert_eq!(clock(245.0), "4:05");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-5.0), "0:00");
    }

    #[test]
    fn lists_are_numbered_from_one() {
        assert_eq!(
            numbered(&["Main".to_owned(), "untitled".to_owned()]),
            "1. Main, 2. untitled"
        );
    }
}
