//! The one-way hash that is all the database keeps of a secret.
//!
//! Tokens are 256 bits of randomness, so a plain SHA-256 is the right hash for them: no slow
//! hash would make guessing one any harder. Pairing codes are only about 40 bits, so their
//! SHA-256 can be reversed by brute force within a minute by someone holding a copy of the
//! database; what protects them is that a code works once, for ten minutes, and that whoever
//! can read the live database can usually write it too. Secrets are looked up by their hash,
//! so no secret is compared in memory and nothing here needs a constant-time comparison; to
//! keep it that way, the secret types have no `==` at all.

use std::fmt;

use sha2::{Digest, Sha256};

/// The SHA-256 of a secret. Compare [`as_bytes`](Self::as_bytes) where a test needs to.
#[derive(Clone, Copy)]
pub struct SecretHash([u8; 32]);

impl SecretHash {
    /// The hash of `secret`'s UTF-8 bytes.
    ///
    /// ```
    /// use emberhut_core::SecretHash;
    ///
    /// assert_eq!(SecretHash::of("abc").as_bytes(), SecretHash::of("abc").as_bytes());
    /// assert_ne!(SecretHash::of("abc").as_bytes(), SecretHash::of("abd").as_bytes());
    /// ```
    #[must_use]
    pub fn of(secret: &str) -> Self {
        Self(Sha256::digest(secret.as_bytes()).into())
    }

    /// The hash of a secret made of raw bytes (a link's nonce).
    #[must_use]
    pub fn of_bytes(secret: &[u8]) -> Self {
        Self(Sha256::digest(secret).into())
    }

    /// The hash stored as `bytes`; `None` unless they are 32.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    /// The hash's 32 bytes, as they are stored.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SecretHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Not the secret, but nothing in it is worth a log line either.
        f.write_str("SecretHash(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_sha256() {
        assert_eq!(
            SecretHash::of("abc").as_bytes()[..4],
            [0xba, 0x78, 0x16, 0xbf]
        );
    }

    #[test]
    fn debug_shows_nothing() {
        assert_eq!(format!("{:?}", SecretHash::of("abc")), "SecretHash(..)");
    }
}
