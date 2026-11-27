//! What an approver is shown, made honest: characters that change how text reads without
//! showing themselves (bidirectional overrides and isolates, zero-width characters, the other
//! format characters of Unicode's `Cf` category) are written out as `[U+XXXX]`, so a target or
//! text can't show one thing and say another. A file's name may not hold one at all.

use std::borrow::Cow;
use std::fmt::Write as _;

/// Unicode's format characters (general category `Cf`), as of Unicode 16.
const FORMAT: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Whether `c` changes how text reads without showing itself: a format character (`Cf`).
///
/// ```
/// use emberhut_core::approvals::shown::is_invisible;
///
/// assert!(is_invisible('\u{202E}'));
/// assert!(is_invisible('\u{200B}'));
/// assert!(!is_invisible('a'));
/// ```
#[must_use]
pub fn is_invisible(c: char) -> bool {
    let code = u32::from(c);
    FORMAT
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}

/// `text` with every invisible character written out as `[U+XXXX]`.
///
/// ```
/// use emberhut_core::approvals::shown::visible;
///
/// assert_eq!(visible("example.com/\u{202E}gol"), "example.com/[U+202E]gol");
/// assert_eq!(visible("plain"), "plain");
/// ```
#[must_use]
pub fn visible(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_invisible) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        if is_invisible(c) {
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
    fn overrides_isolates_and_zero_widths_are_written_out() {
        assert_eq!(
            visible("a\u{2066}b\u{200D}c\u{FEFF}"),
            "a[U+2066]b[U+200D]c[U+FEFF]"
        );
        assert_eq!(visible("tag\u{E0041}"), "tag[U+E0041]");
        assert!(!is_invisible('\n') && !is_invisible('é') && !is_invisible('\u{0627}'));
    }
}
