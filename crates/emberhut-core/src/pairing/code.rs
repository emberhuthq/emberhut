//! Pairing codes: eight characters a person can read off one screen and type on another.

use std::fmt;

use crate::random::{self, RandomError, RandomSource};
use crate::secret::SecretHash;

/// The characters a code is made of: capitals and digits without look-alikes (no 0 or O, no
/// 1, I or L). 31 of them, so a code is one of 31^8 (about 850 billion).
const ALPHABET: &[u8; 31] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// Characters in a code.
const LENGTH: usize = 8;
/// A code is shown in groups of this many, with a dash between.
const GROUP: usize = 4;
/// Random bytes below this map evenly onto the alphabet (8 × 31); the rest are drawn again.
const UNBIASED_BELOW: u8 = 248;
/// Batches of random bytes drawn before giving up on a source. A working source fails a
/// batch (fewer than 8 usable bytes in 16) about once in 10^13 batches.
const MAX_DRAWS: usize = 32;

/// A pairing code that was just made. It is shown once, as `ABCD-EFGH`, and only its hash is
/// kept. `Debug` does not print it, and it has no `==`: secrets are compared by their hash,
/// in the database.
#[derive(Clone)]
pub struct PairingCode(String);

impl PairingCode {
    /// A new code, each character drawn evenly from the alphabet.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when `random` fails, or yields almost nothing but bytes that cannot be
    /// used without bias.
    pub fn generate(random: &dyn RandomSource) -> Result<Self, RandomError> {
        let mut code = String::with_capacity(LENGTH);
        for _ in 0..MAX_DRAWS {
            for byte in random::bytes::<16>(random)? {
                if byte < UNBIASED_BELOW && code.len() < LENGTH {
                    code.push(char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]));
                }
            }
            if code.len() == LENGTH {
                return Ok(Self(code));
            }
        }
        Err(RandomError::new(BiasedSource))
    }

    /// What the database keeps of this code.
    #[must_use]
    pub fn hash(&self) -> SecretHash {
        SecretHash::of(&self.0)
    }
}

/// The code as it is shown: `ABCD-EFGH`.
impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (first, second) = self.0.split_at(GROUP);
        write!(f, "{first}-{second}")
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(..)")
    }
}

/// A random source that yielded almost only bytes a code cannot use.
#[derive(Debug, thiserror::Error)]
#[error("the random source yielded almost no usable bytes")]
struct BiasedSource;

/// A code as someone typed it, written the way codes are hashed: capitals, without the dash,
/// spaces or anything else that is not a letter or digit. `Debug` does not print it.
#[derive(Clone)]
pub struct TypedCode(String);

impl TypedCode {
    /// The code in `typed`; `None` when it has no letters or digits at all.
    ///
    /// ```
    /// use emberhut_core::pairing::TypedCode;
    ///
    /// let typed = TypedCode::parse("abcd efgh").unwrap();
    /// let shown = TypedCode::parse("ABCD-EFGH").unwrap();
    /// assert_eq!(typed.hash().as_bytes(), shown.hash().as_bytes());
    /// assert!(TypedCode::parse(" - ").is_none());
    /// ```
    #[must_use]
    pub fn parse(typed: &str) -> Option<Self> {
        let code: String = typed
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .collect();
        (!code.is_empty()).then_some(Self(code))
    }

    /// The hash to look the code up by.
    #[must_use]
    pub fn hash(&self) -> SecretHash {
        SecretHash::of(&self.0)
    }
}

impl fmt::Debug for TypedCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TypedCode(..)")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::random::OsRandom;

    /// Yields `script` over and over.
    #[derive(Debug)]
    struct Scripted {
        script: Vec<u8>,
        next: AtomicUsize,
    }

    impl Scripted {
        fn new(script: &[u8]) -> Self {
            Self {
                script: script.to_vec(),
                next: AtomicUsize::new(0),
            }
        }
    }

    impl RandomSource for Scripted {
        fn fill(&self, bytes: &mut [u8]) -> Result<(), RandomError> {
            for byte in bytes {
                let at = self.next.fetch_add(1, Ordering::Relaxed);
                *byte = self.script[at % self.script.len()];
            }
            Ok(())
        }
    }

    #[test]
    fn code_is_eight_characters_from_the_alphabet() {
        for _ in 0..200 {
            let code = PairingCode::generate(&OsRandom).unwrap();
            assert_eq!(code.0.len(), LENGTH);
            assert!(code.0.bytes().all(|b| ALPHABET.contains(&b)), "{}", code.0);
        }
    }

    #[test]
    fn bytes_map_onto_the_alphabet_and_the_biased_ones_are_skipped() {
        // 248 and 255 would favour the alphabet's first characters, so they are drawn again;
        // 0 is 'A', 30 is '9' (the last), 31 wraps to 'A', 247 is 247 % 31 = 30, '9' again.
        let random = Scripted::new(&[248, 255, 0, 30, 31, 247, 1, 2, 3, 29]);
        let code = PairingCode::generate(&random).unwrap();
        assert_eq!(code.0, "A9A9BCD8");
    }

    #[test]
    fn a_source_of_only_unusable_bytes_is_an_error_not_a_hang() {
        let random = Scripted::new(&[255]);
        assert!(PairingCode::generate(&random).is_err());
    }

    #[test]
    fn alphabet_has_no_look_alikes_or_repeats() {
        for look_alike in b"01OIL" {
            assert!(!ALPHABET.contains(look_alike));
        }
        let mut sorted = ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ALPHABET.len());
        assert_eq!(usize::from(UNBIASED_BELOW), ALPHABET.len() * 8);
    }

    #[test]
    fn shown_in_two_groups() {
        let code = PairingCode("ABCDEFGH".to_owned());
        assert_eq!(code.to_string(), "ABCD-EFGH");
        assert_eq!(format!("{code:?}"), "PairingCode(..)");
    }

    #[test]
    fn typed_code_matches_however_it_is_written() {
        let code = PairingCode("ABCDEFGH".to_owned());
        for typed in ["ABCD-EFGH", "abcd efgh", " abCD-efGH\n", "ABCDEFGH"] {
            let typed = TypedCode::parse(typed).unwrap();
            assert_eq!(typed.hash().as_bytes(), code.hash().as_bytes());
            assert_eq!(format!("{typed:?}"), "TypedCode(..)");
        }
        assert!(TypedCode::parse(" - ").is_none());
        assert!(TypedCode::parse("").is_none());
    }
}
