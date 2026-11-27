//! The two text encodings of binary values: base64url without padding (keys, nonces,
//! signatures) and lowercase base32 without padding (the machine id).
//!
//! Both decoders accept exactly one spelling of each value: no padding, no whitespace, no other
//! alphabet, and no non-zero trailing bits. So two texts that differ are two values, and a value
//! can be compared as text.

use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD, DecodeKind};

/// Why a text isn't the encoding of a value of the expected size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The text has the wrong number of characters for the value.
    #[error("expected {expected} characters, found {found}")]
    Length {
        /// How many characters a value of this kind takes.
        expected: usize,
        /// How many the text had.
        found: usize,
    },
    /// A character outside the alphabet, padding included.
    #[error("character {position} is not in the alphabet")]
    Symbol {
        /// Where, counting bytes from 0.
        position: usize,
    },
    /// The last character carries bits that aren't zero, so the value has another spelling.
    #[error("the last character has non-zero trailing bits")]
    TrailingBits,
}

impl DecodeError {
    fn from_data_encoding(error: data_encoding::DecodeError) -> Self {
        match error.kind {
            DecodeKind::Trailing => Self::TrailingBits,
            // Length was checked first, so what is left is a character out of place.
            DecodeKind::Symbol | DecodeKind::Padding | DecodeKind::Length => Self::Symbol {
                position: error.position,
            },
        }
    }
}

/// `bytes` as base64url without padding (RFC 4648, section 5).
pub(crate) fn base64url(bytes: &[u8]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

/// The `N` bytes that `text` spells in base64url without padding.
pub(crate) fn from_base64url<const N: usize>(text: &str) -> Result<[u8; N], DecodeError> {
    decode_exact(&BASE64URL_NOPAD, text.as_bytes())
}

/// `bytes` as lowercase base32 without padding (RFC 4648, section 6, in lowercase).
pub(crate) fn base32_lower(bytes: &[u8]) -> String {
    let mut text = BASE32_NOPAD.encode(bytes);
    text.make_ascii_lowercase();
    text
}

/// The `N` bytes that `text` spells in lowercase base32 without padding. Upper case is refused:
/// the id has one spelling.
pub(crate) fn from_base32_lower<const N: usize>(text: &str) -> Result<[u8; N], DecodeError> {
    if let Some(position) = text.bytes().position(|byte| byte.is_ascii_uppercase()) {
        return Err(DecodeError::Symbol { position });
    }
    decode_exact(&BASE32_NOPAD, text.to_ascii_uppercase().as_bytes())
}

fn decode_exact<const N: usize>(
    encoding: &data_encoding::Encoding,
    text: &[u8],
) -> Result<[u8; N], DecodeError> {
    let expected = encoding.encode_len(N);
    if text.len() != expected {
        return Err(DecodeError::Length {
            expected,
            found: text.len(),
        });
    }
    let mut out = [0_u8; N];
    encoding
        .decode_mut(text, &mut out)
        .map_err(|partial| DecodeError::from_data_encoding(partial.error))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn base64url_has_one_spelling() {
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(from_base64url::<2>("-_8"), Ok([0xfb, 0xff]));
        assert_eq!(
            from_base64url::<2>("-_9"),
            Err(DecodeError::TrailingBits),
            "the same bytes with a trailing bit set"
        );
        assert_eq!(
            from_base64url::<2>("+/8"),
            Err(DecodeError::Symbol { position: 0 }),
            "the standard alphabet is not base64url"
        );
        assert_eq!(
            from_base64url::<2>("-_8="),
            Err(DecodeError::Length {
                expected: 3,
                found: 4
            }),
            "no padding"
        );
        assert_eq!(
            from_base64url::<1>("A="),
            Err(DecodeError::Symbol { position: 1 })
        );
    }

    #[test]
    fn base32_is_lowercase_only() {
        assert_eq!(base32_lower(b"ember"), "mvwwezls");
        assert_eq!(from_base32_lower::<5>("mvwwezls"), Ok(*b"ember"));
        assert_eq!(
            from_base32_lower::<5>("MVWWEZLS"),
            Err(DecodeError::Symbol { position: 0 })
        );
        assert_eq!(
            from_base32_lower::<5>("mvwwEzls"),
            Err(DecodeError::Symbol { position: 4 })
        );
        assert_eq!(
            from_base32_lower::<5>("mvwwezl0"),
            Err(DecodeError::Symbol { position: 7 }),
            "0 is not in the alphabet"
        );
    }

    proptest! {
        #[test]
        fn base64url_round_trips(bytes in any::<[u8; 32]>()) {
            let text = base64url(&bytes);
            prop_assert_eq!(text.len(), 43);
            prop_assert_eq!(from_base64url::<32>(&text), Ok(bytes));
        }

        #[test]
        fn base32_round_trips(bytes in any::<[u8; 20]>()) {
            let text = base32_lower(&bytes);
            prop_assert_eq!(text.len(), 32);
            prop_assert!(text.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)));
            prop_assert_eq!(from_base32_lower::<20>(&text), Ok(bytes));
        }

        #[test]
        fn base64url_decoding_never_panics(text in "\\PC{0,90}") {
            let _decoded = from_base64url::<32>(&text);
        }
    }
}
