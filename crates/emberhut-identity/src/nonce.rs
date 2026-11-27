//! Nonces: 32 fresh random bytes that make a signed answer unique to one request.

use std::fmt;
use std::str::FromStr;

use crate::encoding::{DecodeError, base64url, from_base64url};
use crate::random::{RandomError, fill_random};

/// 32 random bytes, sent by whoever wants a fresh signature (a device probing a route, the
/// relay greeting a machine). Its text is 43 characters of base64url without padding.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Nonce([u8; Nonce::LEN]);

impl Nonce {
    /// A nonce's length in bytes.
    pub const LEN: usize = 32;

    /// A fresh nonce from the operating system's random source.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the operating system can't provide randomness.
    pub fn generate() -> Result<Self, RandomError> {
        let mut bytes = [0_u8; Self::LEN];
        fill_random(&mut bytes)?;
        Ok(Self(bytes))
    }

    /// The nonce whose bytes are `bytes`.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The nonce's bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Display for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&base64url(&self.0))
    }
}

impl fmt::Debug for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Nonce({self})")
    }
}

impl FromStr for Nonce {
    type Err = DecodeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        from_base64url(text).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_fresh_and_have_text() {
        let one = Nonce::generate().unwrap();
        assert_ne!(one, Nonce::generate().unwrap());
        assert_eq!(one.to_string().len(), 43);
        assert_eq!(one.to_string().parse::<Nonce>(), Ok(one));
        assert_eq!(
            Nonce::from_bytes([0; 32]).to_string(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        );
    }
}
