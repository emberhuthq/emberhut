//! What an approval covers, exactly: the payload and its hash.
//!
//! The hash is SHA-256 over a canonical encoding: the version tag `emberhut-approval-v1`, the
//! agent, the kind, the target, the text, the amount, and the files sorted by their SHA-256.
//! Every variable-length part is prefixed with its length (a big-endian `u64`), optional parts
//! with a presence byte, and the file list with its count, so no two different payloads share
//! an encoding.
//!
//! Each file contributes its SHA-256 (the bytes, read by the server, never the agent's word),
//! its size and its name. The name is covered because the approver is shown it and may judge
//! by it (`invoice-final.pdf` against `invoice-draft.pdf`): a renamed file is a new request.
//! A payment's amount is covered because it is what the approver says yes to; the same text
//! with another amount is a new request.

use std::fmt;

use data_encoding::{HEXLOWER, HEXLOWER_PERMISSIVE};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;

use super::kind::ActionKind;
use crate::agents::AgentId;

/// The version tag that starts every encoding: a new encoding gets a new tag.
pub const PAYLOAD_TAG: &str = "emberhut-approval-v1";

/// A SHA-256 digest: of a file's bytes, or of a payload.
#[derive(Clone, Copy)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// The SHA-256 of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The digest stored as `bytes`; none unless they are 32.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    /// The digest written as 64 hex digits (either case); none for anything else.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 64 {
            return None;
        }
        HEXLOWER_PERMISSIVE
            .decode(hex.as_bytes())
            .ok()
            .and_then(|bytes| Self::from_bytes(&bytes))
    }

    /// Its 32 bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Its 64 lower-case hex digits, as an approver is shown it.
    #[must_use]
    pub fn to_hex(&self) -> String {
        HEXLOWER.encode(&self.0)
    }
}

/// Compared in constant time: a digest an outsider sends is compared with one the server made.
impl PartialEq for Sha256Digest {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl Eq for Sha256Digest {}

impl std::hash::Hash for Sha256Digest {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialOrd for Sha256Digest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Byte order, which is the order of their hex: the order files are hashed in.
impl Ord for Sha256Digest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Digest({})", self.to_hex())
    }
}

/// A payload's hash: what a decision, a link and a use each carry.
pub type PayloadHash = Sha256Digest;

/// One file of a payload, as the server read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDigest {
    /// Its name, as the approver is shown it.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// The SHA-256 of its bytes.
    pub sha256: Sha256Digest,
}

impl FileDigest {
    /// The digest of a file called `name` holding `bytes`.
    #[must_use]
    pub fn of(name: impl Into<String>, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: Sha256Digest::of(bytes),
        }
    }
}

/// Exactly what is to happen, as far as an approval covers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload<'a> {
    /// The agent that acts.
    pub agent: &'a AgentId,
    /// The kind of action, after the fixed core's raise.
    pub kind: ActionKind,
    /// Where it goes: a channel, a repository and branch, a URL, an account.
    pub target: &'a str,
    /// The exact words, if any.
    pub text: Option<&'a str>,
    /// A payment's amount, in cents.
    pub amount_cents: Option<u64>,
    /// Its files.
    pub files: &'a [FileDigest],
}

impl Payload<'_> {
    /// The canonical encoding the hash is taken over.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put(&mut out, PAYLOAD_TAG.as_bytes());
        put(&mut out, self.agent.as_str().as_bytes());
        put(&mut out, self.kind.as_str().as_bytes());
        put(&mut out, self.target.as_bytes());
        match self.text {
            Some(text) => {
                out.push(1);
                put(&mut out, text.as_bytes());
            }
            None => out.push(0),
        }
        match self.amount_cents {
            Some(cents) => {
                out.push(1);
                out.extend_from_slice(&cents.to_be_bytes());
            }
            None => out.push(0),
        }
        let mut files: Vec<&FileDigest> = self.files.iter().collect();
        files.sort_by(|a, b| {
            (a.sha256, a.name.as_str(), a.size).cmp(&(b.sha256, b.name.as_str(), b.size))
        });
        out.extend_from_slice(&count(files.len()).to_be_bytes());
        for file in files {
            out.extend_from_slice(file.sha256.as_bytes());
            out.extend_from_slice(&file.size.to_be_bytes());
            put(&mut out, file.name.as_bytes());
        }
        out
    }

    /// The payload's hash.
    ///
    /// ```
    /// use emberhut_core::agents::AgentId;
    /// use emberhut_core::approvals::{ActionKind, Payload};
    ///
    /// let agent = AgentId::parse("client-a").unwrap();
    /// let post = |text| Payload {
    ///     agent: &agent,
    ///     kind: ActionKind::Post,
    ///     target: "example blog",
    ///     text: Some(text),
    ///     amount_cents: None,
    ///     files: &[],
    /// };
    /// assert_eq!(post("Hello").hash(), post("Hello").hash());
    /// assert_ne!(post("Hello").hash(), post("Hello!").hash());
    /// ```
    #[must_use]
    pub fn hash(&self) -> PayloadHash {
        Sha256Digest::of(&self.encode())
    }
}

/// `n` as the encoding writes a count or a length.
pub(super) fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// `bytes`, prefixed with their length.
pub(super) fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&count(bytes.len()).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> AgentId {
        AgentId::parse("client-a").unwrap()
    }

    fn files() -> Vec<FileDigest> {
        vec![
            FileDigest::of("cover.png", b"png bytes"),
            FileDigest::of("post.md", b"markdown bytes"),
        ]
    }

    fn base<'a>(agent: &'a AgentId, files: &'a [FileDigest]) -> Payload<'a> {
        Payload {
            agent,
            kind: ActionKind::Post,
            target: "example blog",
            text: Some("We shipped it."),
            amount_cents: None,
            files,
        }
    }

    #[test]
    fn the_same_payload_hashes_the_same() {
        let (agent, files) = (agent(), files());
        assert_eq!(base(&agent, &files).hash(), base(&agent, &files).hash());
    }

    #[test]
    fn file_order_does_not_matter() {
        let agent = agent();
        let files = files();
        let reversed: Vec<_> = files.iter().rev().cloned().collect();
        assert_eq!(base(&agent, &files).hash(), base(&agent, &reversed).hash());
    }

    #[test]
    fn every_part_changes_the_hash() {
        let (agent, files) = (agent(), files());
        let other_agent = AgentId::parse("client-b").unwrap();
        let original = base(&agent, &files).hash();
        let mut renamed = files.clone();
        renamed[0].name = "cover-final.png".to_owned();
        let mut edited = files.clone();
        edited[1] = FileDigest::of("post.md", b"markdown bytes, edited");
        let fewer = &files[..1];
        let changed = [
            Payload {
                text: Some("We shipped it!"),
                ..base(&agent, &files)
            },
            Payload {
                text: None,
                ..base(&agent, &files)
            },
            Payload {
                target: "example forum",
                ..base(&agent, &files)
            },
            Payload {
                kind: ActionKind::Send,
                ..base(&agent, &files)
            },
            Payload {
                amount_cents: Some(100),
                ..base(&agent, &files)
            },
            base(&other_agent, &files),
            base(&agent, &renamed),
            base(&agent, &edited),
            base(&agent, fewer),
        ];
        for payload in changed {
            assert_ne!(payload.hash(), original, "{payload:?}");
        }
    }

    #[test]
    fn no_text_and_empty_text_differ() {
        let agent = agent();
        let none = Payload {
            text: None,
            ..base(&agent, &[])
        };
        let empty = Payload {
            text: Some(""),
            ..base(&agent, &[])
        };
        assert_ne!(none.hash(), empty.hash());
    }

    #[test]
    fn length_prefixes_keep_fields_apart() {
        let agent = agent();
        let a = Payload {
            target: "ab",
            text: Some("c"),
            ..base(&agent, &[])
        };
        let b = Payload {
            target: "a",
            text: Some("bc"),
            ..base(&agent, &[])
        };
        assert_ne!(a.encode(), b.encode());
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn the_encoding_starts_with_its_tag() {
        let agent = agent();
        let encoded = base(&agent, &[]).encode();
        let length = PAYLOAD_TAG.len();
        assert_eq!(&encoded[..8], &(length as u64).to_be_bytes());
        assert_eq!(&encoded[8..8 + length], PAYLOAD_TAG.as_bytes());
    }

    #[test]
    fn hex_round_trips() {
        let digest = Sha256Digest::of(b"abc");
        assert_eq!(
            digest.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(Sha256Digest::from_hex(&digest.to_hex()), Some(digest));
        assert_eq!(
            Sha256Digest::from_hex(&digest.to_hex().to_uppercase()),
            Some(digest)
        );
        assert_eq!(Sha256Digest::from_hex("ba78"), None);
        assert_eq!(Sha256Digest::from_hex(&"g".repeat(64)), None);
        assert_eq!(Sha256Digest::from_bytes(&[0; 31]), None);
    }

    #[test]
    fn file_digests_read_their_bytes() {
        let file = FileDigest::of("a.txt", b"hello");
        assert_eq!(file.size, 5);
        assert_eq!(file.sha256, Sha256Digest::of(b"hello"));
    }
}
