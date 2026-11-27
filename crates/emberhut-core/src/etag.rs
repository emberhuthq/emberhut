//! Entity tags for responses that clients poll, so an unchanged answer costs a `304`.
//!
//! The tag is a digest of the body the server would send: the same bytes give the same tag,
//! on any server and after any restart, and nothing has to be invalidated when something
//! changes. It is weak (`W/"…"`), because a response may be compressed on its way out, and a
//! strong tag promises the same bytes on the wire.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// Digest bytes kept in a tag: 128 bits, far past any chance of two bodies colliding.
const DIGEST_BYTES: usize = 16;

/// A weak entity tag for a response body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ETag(String);

impl ETag {
    /// The tag of `body`.
    ///
    /// ```
    /// use emberhut_core::ETag;
    ///
    /// let tag = ETag::of(br#"{"room":1}"#);
    /// assert!(tag.as_str().starts_with("W/\""));
    /// assert_eq!(tag, ETag::of(br#"{"room":1}"#));
    /// assert!(tag.matches_if_none_match(tag.as_str()));
    /// ```
    #[must_use]
    pub fn of(body: &[u8]) -> Self {
        let digest = Sha256::digest(body);
        Self(format!(
            "W/\"{}\"",
            URL_SAFE_NO_PAD.encode(&digest[..DIGEST_BYTES])
        ))
    }

    /// The tag as an `ETag` header value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether an `If-None-Match` header value names this tag, so the client's copy is
    /// current. The comparison is weak, as RFC 9110 has it for `If-None-Match`: `W/"x"` and
    /// `"x"` match. The header may list several tags, or be `*`.
    #[must_use]
    pub fn matches_if_none_match(&self, header: &str) -> bool {
        let ours = opaque(&self.0);
        header
            .split(',')
            .map(str::trim)
            .any(|tag| tag == "*" || opaque(tag) == ours)
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A tag without its weakness marker: what a weak comparison compares.
fn opaque(tag: &str) -> &str {
    tag.strip_prefix("W/").unwrap_or(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_follows_the_body() {
        assert_eq!(ETag::of(b"a"), ETag::of(b"a"));
        assert_ne!(ETag::of(b"a"), ETag::of(b"b"));
        // "W/" + quotes + 16 bytes in unpadded base64.
        assert_eq!(ETag::of(b"a").as_str().len(), 2 + 2 + 22);
    }

    #[test]
    fn if_none_match_compares_weakly_and_reads_lists() {
        let tag = ETag::of(b"state");
        let strong = tag.as_str().trim_start_matches("W/").to_owned();
        assert!(tag.matches_if_none_match(tag.as_str()));
        assert!(tag.matches_if_none_match(&strong));
        assert!(tag.matches_if_none_match(&format!("W/\"other\", {}", tag.as_str())));
        assert!(tag.matches_if_none_match("*"));
        assert!(!tag.matches_if_none_match("W/\"other\""));
        assert!(!tag.matches_if_none_match(""));
    }
}
