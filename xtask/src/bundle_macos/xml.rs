//! Escaping text for XML.

/// `text` with the characters XML gives a meaning to replaced by entities.
#[must_use]
pub(crate) fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn escape_replaces_the_five_special_characters() {
        assert_eq!(
            escape("<a & 'b' \"c\">"),
            "&lt;a &amp; &apos;b&apos; &quot;c&quot;&gt;"
        );
        assert_eq!(escape("plain"), "plain");
    }
}
