//! The machine id: a name derived from the public key.
//!
//! It is the first 160 bits of `SHA-256(statement("emberhut machine id v1", [public key]))`, in
//! lowercase base32 without padding: 32 characters from `a-z2-7`. That makes it one DNS label
//! (the relay serves the machine at `<id>.<relay domain>`), unchanged by DNS's case folding,
//! never `xn--`, too long to enumerate, and self-certifying: whoever shows the key proves the
//! id.

use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

use crate::encoding::{DecodeError, base32_lower, from_base32_lower};
use crate::keys::PublicKey;
use crate::statement::{Encoder, prefix};

/// A machine's id, derived from its public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MachineId([u8; MachineId::BYTES]);

/// A public key that doesn't have the machine id it was given with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("the key doesn't belong to machine {expected}")]
pub struct WrongKey {
    /// The id the key was meant to have.
    pub expected: MachineId,
}

impl MachineId {
    /// The id's size in bytes: 160 bits.
    pub const BYTES: usize = 20;
    /// The id's length as text, in characters.
    pub const LEN: usize = 32;

    /// The id of `key`.
    #[must_use]
    pub fn of(key: &PublicKey) -> Self {
        let input = Encoder::new(prefix::MACHINE_ID)
            .bytes(key.as_bytes())
            .finish();
        let digest = Sha256::digest(input);
        let mut id = [0_u8; Self::BYTES];
        id.copy_from_slice(&digest[..Self::BYTES]);
        Self(id)
    }

    /// The id's 20 bytes. A statement carries the id as its 32 characters of text, not these.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::BYTES] {
        &self.0
    }

    /// Checks that `key` has this id.
    ///
    /// # Errors
    ///
    /// [`WrongKey`] when it has another.
    pub fn check(&self, key: &PublicKey) -> Result<(), WrongKey> {
        if Self::of(key) == *self {
            Ok(())
        } else {
            Err(WrongKey { expected: *self })
        }
    }
}

impl fmt::Display for MachineId {
    /// 32 characters of lowercase base32.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&base32_lower(&self.0))
    }
}

impl fmt::Debug for MachineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MachineId({self})")
    }
}

impl FromStr for MachineId {
    type Err = DecodeError;

    /// Reads exactly 32 characters from `a-z2-7`: upper case and padding are refused.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        from_base32_lower(text).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::SecretKey;

    #[test]
    fn the_id_is_a_dns_label() {
        let id = SecretKey::from_bytes(&[1; 32]).machine_id();
        let text = id.to_string();
        assert_eq!(text.len(), MachineId::LEN);
        assert!(!text.starts_with("xn--"));
        assert_eq!(text.parse::<MachineId>(), Ok(id));
        assert_eq!(format!("{id:?}"), format!("MachineId({text})"));
    }

    #[test]
    fn a_key_proves_only_its_own_id() {
        let one = SecretKey::from_bytes(&[1; 32]).public_key();
        let two = SecretKey::from_bytes(&[2; 32]).public_key();
        assert_eq!(one.machine_id().check(&one), Ok(()));
        assert_eq!(
            one.machine_id().check(&two),
            Err(WrongKey {
                expected: one.machine_id()
            })
        );
    }

    #[test]
    fn only_the_lowercase_spelling_reads() {
        let text = SecretKey::from_bytes(&[1; 32]).machine_id().to_string();
        assert!(text.to_ascii_uppercase().parse::<MachineId>().is_err());
        assert!(text[1..].parse::<MachineId>().is_err());
        assert!(format!("{text}=").parse::<MachineId>().is_err());
    }

    proptest! {
        #[test]
        fn ids_round_trip(bytes in any::<[u8; 20]>()) {
            let id = MachineId(bytes);
            prop_assert_eq!(id.to_string().parse::<MachineId>(), Ok(id));
        }

        #[test]
        fn id_parsing_never_panics(text in "\\PC{0,40}") {
            let _id = text.parse::<MachineId>();
        }
    }
}
