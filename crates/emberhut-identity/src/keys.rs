//! The machine's Ed25519 key pair and its signatures.
//!
//! The secret key's bytes are wiped when it is dropped, and its `Debug` shows only the public
//! key.
//!
//! Verification is one rule, which `docs/api.md` states for every implementation: `s < L`; `A`
//! and `R` canonical encodings of points in the prime-order subgroup, neither of small order;
//! and `[s]B = R + [k]A` without the cofactor, `k` being `SHA-512(R || A || M)` mod `L`. A key
//! is checked when it is read ([`PublicKey::from_bytes`]); the rest is ed25519-dalek's strict
//! verification. So whether a signature verifies never depends on the implementation.

use std::fmt;
use std::str::FromStr;

use ed25519_dalek::{SigningKey, VerifyingKey};
use subtle::{Choice, ConstantTimeEq};
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::encoding::{DecodeError, base64url, from_base64url};
use crate::machine_id::MachineId;
use crate::random::{RandomError, fill_random};
use crate::statement::Statement;

/// The length of a secret key (an RFC 8032 seed), in bytes.
pub const SECRET_KEY_LEN: usize = 32;
/// The length of a public key, in bytes.
pub const PUBLIC_KEY_LEN: usize = 32;
/// The length of a signature, in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// A machine's (or a device's) secret Ed25519 key.
///
/// It has no `Clone`, `Display` or serde form: its bytes leave only through
/// [`SecretKey::to_bytes`], in a buffer that is wiped when dropped, to be written to the key
/// file. Comparing two keys takes the same time wherever they differ.
pub struct SecretKey(SigningKey);

impl SecretKey {
    /// A new key from the operating system's random source.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the operating system can't provide randomness.
    pub fn generate() -> Result<Self, RandomError> {
        let mut seed = Zeroizing::new([0_u8; SECRET_KEY_LEN]);
        fill_random(seed.as_mut())?;
        Ok(Self::from_bytes(&seed))
    }

    /// The key whose RFC 8032 seed is `seed`, as read back from the key file (or a test
    /// vector).
    #[must_use]
    pub fn from_bytes(seed: &[u8; SECRET_KEY_LEN]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }

    /// The seed, to keep in the key file. The buffer is wiped when dropped.
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<[u8; SECRET_KEY_LEN]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The public half.
    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    /// The machine id of the public half.
    #[must_use]
    pub fn machine_id(&self) -> MachineId {
        MachineId::of(&self.public_key())
    }

    /// Signs a statement.
    #[must_use]
    pub fn sign<S: Statement>(&self, statement: &S) -> Signature {
        use ed25519_dalek::Signer as _;
        Signature(self.0.sign(&statement.to_bytes()))
    }
}

impl ConstantTimeEq for SecretKey {
    fn ct_eq(&self, other: &Self) -> Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for SecretKey {
    /// In constant time.
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other).into()
    }
}

impl Eq for SecretKey {}

// The inner `SigningKey` wipes its bytes when it is dropped.
impl ZeroizeOnDrop for SecretKey {}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKey")
            .field("public_key", &self.public_key())
            .finish_non_exhaustive()
    }
}

/// An Ed25519 public key: the machine's, or a device's.
///
/// Its text is 43 characters of base64url without padding. Only canonical encodings of points
/// in the prime-order subgroup that aren't of small order are accepted.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(VerifyingKey);

/// Why bytes or text aren't a public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// The text isn't 32 bytes in base64url without padding.
    #[error("not a public key's text")]
    Decode(#[from] DecodeError),
    /// The bytes aren't the canonical encoding of a curve point.
    #[error("not a public key: not the canonical encoding of a curve point")]
    NotAPoint,
    /// The point has small order: a signature could be valid for almost any statement.
    #[error("not a public key: a weak (small-order) point")]
    Weak,
    /// The point has a small-order component: it isn't in the prime-order subgroup.
    #[error("not a public key: not in the prime-order subgroup")]
    NotTorsionFree,
}

impl PublicKey {
    /// The key whose bytes are `bytes`.
    ///
    /// # Errors
    ///
    /// [`KeyError::NotAPoint`] for bytes that aren't a canonical point encoding,
    /// [`KeyError::Weak`] for a small-order point, and [`KeyError::NotTorsionFree`] for a point
    /// outside the prime-order subgroup.
    pub fn from_bytes(bytes: &[u8; PUBLIC_KEY_LEN]) -> Result<Self, KeyError> {
        let key = VerifyingKey::from_bytes(bytes).map_err(|_| KeyError::NotAPoint)?;
        // Decompression accepts a few non-canonical encodings; one key has one spelling.
        if key.to_edwards().compress().to_bytes() != *bytes {
            return Err(KeyError::NotAPoint);
        }
        if key.is_weak() {
            return Err(KeyError::Weak);
        }
        if !key.to_edwards().is_torsion_free() {
            return Err(KeyError::NotTorsionFree);
        }
        Ok(Self(key))
    }

    /// The key's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PUBLIC_KEY_LEN] {
        self.0.to_bytes()
    }

    /// The key's bytes, borrowed.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LEN] {
        self.0.as_bytes()
    }

    /// The machine id this key proves.
    #[must_use]
    pub fn machine_id(&self) -> MachineId {
        MachineId::of(self)
    }

    /// Checks that `signature` is this key's over `statement`.
    ///
    /// # Errors
    ///
    /// [`SignatureError`] when it isn't (strict verification: non-canonical signatures and
    /// small-order points are refused too).
    pub fn verify<S: Statement>(
        &self,
        statement: &S,
        signature: &Signature,
    ) -> Result<(), SignatureError> {
        self.verify_message(&statement.to_bytes(), signature)
    }

    /// The verification rule over raw bytes. Only statements are verified outside this crate.
    pub(crate) fn verify_message(
        &self,
        message: &[u8],
        signature: &Signature,
    ) -> Result<(), SignatureError> {
        self.0
            .verify_strict(message, &signature.0)
            .map_err(|_| SignatureError)
    }
}

impl fmt::Display for PublicKey {
    /// 43 characters of base64url without padding.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&base64url(self.as_bytes()))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

impl FromStr for PublicKey {
    type Err = KeyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::from_bytes(&from_base64url(text)?)
    }
}

/// An Ed25519 signature over a [`Statement`].
///
/// Its text is 86 characters of base64url without padding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(ed25519_dalek::Signature);

/// A signature doesn't verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("the signature doesn't verify")]
pub struct SignatureError;

impl Signature {
    /// The signature whose bytes are `bytes`. Any 64 bytes make one; whether it verifies is
    /// [`PublicKey::verify`]'s question.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; SIGNATURE_LEN]) -> Self {
        Self(ed25519_dalek::Signature::from_bytes(bytes))
    }

    /// The signature's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; SIGNATURE_LEN] {
        self.0.to_bytes()
    }
}

impl fmt::Display for Signature {
    /// 86 characters of base64url without padding.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&base64url(&self.to_bytes()))
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({self})")
    }
}

impl FromStr for Signature {
    type Err = DecodeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_bytes(&from_base64url(text)?))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::encoding::from_base64url;
    use crate::{RelayDomain, UnixMillis, UnlinkStatement};

    /// RFC 8032, section 7.1, test 1.
    const RFC_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const RFC_PUBLIC: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];

    fn statement(time: u64) -> UnlinkStatement {
        let key = SecretKey::from_bytes(&RFC_SEED);
        UnlinkStatement::new(
            key.machine_id(),
            "relay.example".parse::<RelayDomain>().unwrap(),
            UnixMillis::new(time).unwrap(),
        )
    }

    #[test]
    fn a_seed_gives_the_rfc_8032_public_key() {
        let key = SecretKey::from_bytes(&RFC_SEED);
        assert_eq!(key.public_key().to_bytes(), RFC_PUBLIC);
        assert_eq!(*key.to_bytes(), RFC_SEED);
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let key = SecretKey::from_bytes(&RFC_SEED);
        let shown = format!("{key:?}");
        assert_eq!(
            shown,
            "SecretKey { public_key: PublicKey(11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo), .. }"
        );
        assert!(!shown.contains(&base64url(&RFC_SEED)), "{shown}");
    }

    #[test]
    fn generated_keys_differ() {
        let one = SecretKey::generate().unwrap();
        let two = SecretKey::generate().unwrap();
        assert_ne!(one, two);
        assert_ne!(one.public_key(), two.public_key());
        assert_eq!(one, SecretKey::from_bytes(&one.to_bytes()));
    }

    #[test]
    fn a_signature_verifies_only_for_its_statement_and_key() {
        let key = SecretKey::from_bytes(&RFC_SEED);
        let signature = key.sign(&statement(1));
        assert_eq!(key.public_key().verify(&statement(1), &signature), Ok(()));
        assert_eq!(
            key.public_key().verify(&statement(2), &signature),
            Err(SignatureError)
        );
        let other = SecretKey::from_bytes(&[7; 32]).public_key();
        assert_eq!(other.verify(&statement(1), &signature), Err(SignatureError));
        let mut bytes = signature.to_bytes();
        bytes[63] ^= 0x80;
        assert_eq!(
            key.public_key()
                .verify(&statement(1), &Signature::from_bytes(&bytes)),
            Err(SignatureError)
        );
    }

    #[test]
    fn public_keys_have_one_spelling() {
        let key = PublicKey::from_bytes(&RFC_PUBLIC).unwrap();
        assert_eq!(key.to_string().parse::<PublicKey>(), Ok(key));
        // The identity point (small order).
        let mut identity = [0_u8; 32];
        identity[0] = 1;
        assert_eq!(PublicKey::from_bytes(&identity), Err(KeyError::Weak));
        // y = p + 1, a non-canonical spelling of the identity's y.
        let mut non_canonical = [0xff_u8; 32];
        non_canonical[0] = 0xee;
        non_canonical[31] = 0x7f;
        assert_eq!(
            PublicKey::from_bytes(&non_canonical),
            Err(KeyError::NotAPoint)
        );
        // An order-8 point: small order.
        let order_8 = [
            0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10,
            0x67, 0x0f, 0x2a, 0x20, 0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77,
            0x92, 0xac, 0x03, 0x7a,
        ];
        assert_eq!(PublicKey::from_bytes(&order_8), Err(KeyError::Weak));
        assert!(matches!(
            "too short".parse::<PublicKey>(),
            Err(KeyError::Decode(DecodeError::Length { .. }))
        ));
    }

    #[test]
    fn signatures_have_text() {
        let key = SecretKey::from_bytes(&RFC_SEED);
        let signature = key.sign(&statement(1));
        let text = signature.to_string();
        assert_eq!(text.len(), 86);
        assert_eq!(text.parse::<Signature>(), Ok(signature));
        assert_eq!(
            from_base64url::<64>(&text).map(|b| Signature::from_bytes(&b)),
            Ok(signature)
        );
        assert_eq!(format!("{signature:?}"), format!("Signature({text})"));
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// The speccheck cases ("Taming the many `EdDSAs`") of the shared vectors, over raw messages, which only
    /// this crate can verify: everywhere else, only statements are.
    #[test]
    fn the_speccheck_cases_follow_the_rule() {
        // emberhut-api holds the shared vectors, beside this crate in the workspace.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("emberhut-api")
            .join("tests")
            .join("fixtures")
            .join("connect")
            .join("signatures.json");
        let vectors: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let cases = vectors["speccheck"].as_array().unwrap();
        assert_eq!(cases.len(), 12);
        for case in cases {
            let text = |key: &str| case[key].as_str().unwrap();
            let key: [u8; 32] = hex(text("publicKey")).try_into().unwrap();
            let signature: [u8; 64] = hex(text("signature")).try_into().unwrap();
            let accepted = PublicKey::from_bytes(&key).is_ok_and(|key| {
                key.verify_message(&hex(text("message")), &Signature::from_bytes(&signature))
                    .is_ok()
            });
            assert_eq!(Some(accepted), case["accept"].as_bool(), "{case}");
        }
    }

    proptest! {
        #[test]
        fn every_seed_signs_and_verifies(seed in any::<[u8; 32]>(), time in 0..=UnixMillis::MAX.get()) {
            let key = SecretKey::from_bytes(&seed);
            let public = key.public_key();
            prop_assert_eq!(PublicKey::from_bytes(&public.to_bytes()), Ok(public));
            let statement = UnlinkStatement::new(
                key.machine_id(),
                "relay.example".parse::<RelayDomain>().unwrap(),
                UnixMillis::new(time).unwrap(),
            );
            prop_assert_eq!(public.verify(&statement, &key.sign(&statement)), Ok(()));
        }

        #[test]
        fn public_key_parsing_never_panics(bytes in any::<[u8; 32]>()) {
            let _key = PublicKey::from_bytes(&bytes);
        }
    }
}
