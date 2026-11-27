//! What the assistant remembers, as rules with no I/O: facts (one small markdown file each, in
//! the context folder), the owner's quick notes and the assistant's diary, the corrections it
//! learns from (standing rules and trial rules), and recall, one search over everything said.
//!
//! The server keeps the files and the rows; these modules decide what they hold and how they
//! read.

pub mod facts;
pub mod learning;
pub mod notes;
pub mod recall;
pub mod wire;

/// `instant` as the wire writes one: `2026-11-24T21:57:53.410Z`.
#[must_use]
pub fn iso(instant: jiff::Timestamp) -> String {
    instant.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The first `limit` characters of `text` (all of it when shorter).
#[must_use]
pub fn cut(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_are_written_with_milliseconds_in_utc() {
        let at: jiff::Timestamp = "2026-11-24T21:57:53.41Z".parse().unwrap();
        assert_eq!(iso(at), "2026-11-24T21:57:53.410Z");
    }

    #[test]
    fn cut_counts_characters() {
        assert_eq!(cut("مرحبا", 2), "مر");
        assert_eq!(cut("ab", 5), "ab");
    }
}
