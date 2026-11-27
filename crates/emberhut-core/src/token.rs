//! Bearer tokens: 256 random bits, written in URL-safe base64.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::random::{self, RandomError, RandomSource};
use crate::secret::SecretHash;

/// A bearer token handed to a device (an access or a refresh token).
///
/// The token is opaque to clients. The server keeps only its [`SecretHash`], so a token that
/// is lost cannot be shown again, only replaced. `Debug` never prints it, and it has no `==`.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    /// A new token from `random`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when `random` fails.
    pub fn generate(random: &dyn RandomSource) -> Result<Self, RandomError> {
        Ok(Self(URL_SAFE_NO_PAD.encode(random::bytes::<32>(random)?)))
    }

    /// The token as the client sends it back.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What the database keeps of this token.
    #[must_use]
    pub fn hash(&self) -> SecretHash {
        SecretHash::of(&self.0)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::OsRandom;

    #[test]
    fn token_is_43_url_safe_characters() {
        let token = Token::generate(&OsRandom).unwrap();
        assert_eq!(token.as_str().len(), 43);
        assert!(
            token
                .as_str()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }

    #[test]
    fn tokens_differ_and_hash_like_their_text() {
        let a = Token::generate(&OsRandom).unwrap();
        let b = Token::generate(&OsRandom).unwrap();
        assert_ne!(a.as_str(), b.as_str());
        assert_eq!(a.hash().as_bytes(), SecretHash::of(a.as_str()).as_bytes());
    }

    #[test]
    fn debug_hides_the_token() {
        let token = Token::generate(&OsRandom).unwrap();
        assert!(!format!("{token:?}").contains(token.as_str()));
    }
}
