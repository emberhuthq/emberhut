//! Query strings: their values percent-encoded so any server's decoder reads them back the same.
//! The client's requests and a link the app builds to another site share this one encoder.

/// Appends `text` to `out` percent-encoded as UTF-8: every byte but RFC 3986's unreserved
/// characters (`A-Z a-z 0-9 - . _ ~`) escaped, so a `+`, `&` or space stays itself.
pub fn push_encoded(out: &mut String, text: &str) {
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0F));
        }
    }
}

/// `text` percent-encoded as [`push_encoded`] does it.
///
/// ```
/// assert_eq!(emberhut_api::query::encoded("a b&c"), "a%20b%26c");
/// ```
#[must_use]
pub fn encoded(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_encoded(&mut out, text);
    out
}

/// The upper-case hex digit for a nibble (`0..=15`).
fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[usize::from(nibble & 0x0F)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_all_but_the_unreserved_characters() {
        let mut out = String::new();
        push_encoded(&mut out, "Room-2 a+b&c~é");
        assert_eq!(out, "Room-2%20a%2Bb%26c~%C3%A9");
    }
}
