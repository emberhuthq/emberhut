//! The one byte encoding of everything a machine signs, and of the machine id's hash input.
//!
//! `statement(prefix, fields)` is:
//!
//! - the ASCII prefix, then a `0x00` byte;
//! - then each field as a 4-byte big-endian length followed by its bytes.
//!
//! A time is 8 bytes (big-endian milliseconds since the Unix epoch), a key or nonce its raw
//! bytes, and text its UTF-8 bytes in its normal form. The lengths keep one field from sliding
//! into the next, and the prefix, different for every kind of statement, keeps a signature made
//! for one purpose from being valid for another.
//!
//! Each kind of statement is a type of this crate that implements [`Statement`]: its prefix and
//! its fields, in order. A new kind is a new type with a prefix of its own, written by the same
//! encoder.

use crate::time::UnixMillis;

/// The domain-separation prefixes, one per kind of statement, as `docs/api.md` lists them.
pub mod prefix {
    /// The machine id's hash input: the public key. Hashed, never signed.
    pub const MACHINE_ID: &str = "emberhut machine id v1";
    /// A probe's answer: nonce, machine id, kind, via, local, host.
    pub const PROBE: &str = "emberhut probe v1";
    /// The relay handshake: relay domain, nonce, time.
    pub const RELAY_AUTH: &str = "emberhut relay auth v1";
    /// Linking a machine to an account: account user id, relay domain, time.
    pub const LINK: &str = "emberhut link v1";
    /// A machine leaving its account: machine id, relay domain, time.
    pub const UNLINK: &str = "emberhut unlink v1";
}

/// Something a machine signs: a prefix and fields, encoded as the module docs say.
///
/// Only this crate's statement types implement it, so every prefix in use is listed in
/// [`prefix`], and nothing else can be signed with a machine's key.
pub trait Statement: sealed::Sealed {
    /// The statement's domain-separation prefix.
    const PREFIX: &'static str;

    /// The bytes that are signed.
    fn to_bytes(&self) -> Vec<u8>;
}

pub(crate) mod sealed {
    /// Keeps [`super::Statement`] to this crate's types.
    pub trait Sealed {}
}

/// The longest field any statement type makes, in bytes: a host with its brackets and port.
const MAX_FIELD: usize = 1024;

/// Writes a statement: the prefix, then fields in order.
#[derive(Debug)]
pub(crate) struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    /// Starts a statement with `prefix` and its `0x00`.
    pub(crate) fn new(prefix: &str) -> Self {
        let mut bytes = Vec::with_capacity(prefix.len() + 128);
        bytes.extend_from_slice(prefix.as_bytes());
        bytes.push(0);
        Self { bytes }
    }

    /// Adds a field of raw bytes. Every field's type bounds it to a few hundred bytes (a host
    /// name to 253, a machine name to 240, an account id to 255), far below what its 4-byte
    /// length holds.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "every field's type bounds it to a few hundred bytes"
    )]
    pub(crate) fn bytes(mut self, field: &[u8]) -> Self {
        debug_assert!(field.len() <= MAX_FIELD, "a field's type bounds its length");
        self.bytes
            .extend_from_slice(&(field.len() as u32).to_be_bytes());
        self.bytes.extend_from_slice(field);
        self
    }

    /// Adds a text field: its UTF-8 bytes.
    pub(crate) fn text(self, field: &str) -> Self {
        self.bytes(field.as_bytes())
    }

    /// Adds a time: 8 bytes, big-endian milliseconds since the Unix epoch.
    pub(crate) fn time(self, time: UnixMillis) -> Self {
        // At most 2^53 - 1, as 8 bytes all the same.
        self.bytes(&time.get().to_be_bytes())
    }

    /// The statement's bytes.
    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_then_zero_then_length_prefixed_fields() {
        let bytes = Encoder::new("p")
            .text("ab")
            .bytes(&[])
            .time(UnixMillis::new(1).unwrap())
            .finish();
        assert_eq!(
            bytes,
            [
                b'p', 0, // the prefix and its terminator
                0, 0, 0, 2, b'a', b'b', // "ab"
                0, 0, 0, 0, // an empty field
                0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 1, // the time 1 ms
            ]
        );
    }

    #[test]
    fn fields_cannot_slide_into_each_other() {
        let one = Encoder::new("p").text("ab").text("c").finish();
        let two = Encoder::new("p").text("a").text("bc").finish();
        assert_ne!(one, two);
    }

    #[test]
    fn every_prefix_is_distinct_ascii_without_a_zero() {
        let all = [
            prefix::MACHINE_ID,
            prefix::PROBE,
            prefix::RELAY_AUTH,
            prefix::LINK,
            prefix::UNLINK,
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(a.is_ascii() && !a.contains('\0'), "{a}");
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
