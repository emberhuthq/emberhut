//! One-time secret links: a page where the person types secrets (an API token, a password)
//! that the assistant asked for, so they land in the server's secrets and never in a chat.
//!
//! The assistant makes a link for a few named secrets and a line saying why; the link itself
//! is the key (192 random bits), works for [`LINK_LIFETIME`], and is gone once the form is
//! sent. Only the link token's hash is stored.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::{SignedDuration, Timestamp};

use crate::ids::SecretLinkId;
use crate::random::{self, RandomError, RandomSource};
use crate::secret::SecretHash;

/// How long a link works: a day.
pub const LINK_LIFETIME: SignedDuration = SignedDuration::from_hours(24);

/// The longest secret name, in bytes.
const NAME_LIMIT: usize = 64;

/// The shortest text a link's path can hold to be looked up at all.
const TOKEN_MIN: usize = 20;

/// A link's token: the last part of its path, `/s/<token>`.
#[derive(Clone)]
pub struct LinkToken(String);

impl LinkToken {
    /// A new token from `random`: 24 bytes in URL-safe base64, 32 characters.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when `random` fails.
    pub fn generate(random: &dyn RandomSource) -> Result<Self, RandomError> {
        Ok(Self(URL_SAFE_NO_PAD.encode(random::bytes::<24>(random)?)))
    }

    /// The token as the link carries it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What the database keeps of it.
    #[must_use]
    pub fn hash(&self) -> SecretHash {
        SecretHash::of(&self.0)
    }
}

impl fmt::Debug for LinkToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LinkToken(..)")
    }
}

/// Whether `text`, a link path's last part, can be a link's token at all: 20 or more ASCII
/// letters, digits, `_` or `-`. Anything else is no link, and is not looked up.
#[must_use]
pub fn looks_like_token(text: &str) -> bool {
    text.len() >= TOKEN_MIN
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The name of a secret, which is also its file's name among the server's secrets: 1 to 64
/// ASCII letters, digits, `.`, `_` or `-`, not starting with a dot.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretName(String);

/// A secret's name that can't be one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{0:?} is not a secret's name: 1 to 64 letters, digits, '.', '_' or '-', not starting with \
     a dot"
)]
pub struct InvalidSecretName(pub String);

impl SecretName {
    /// The name, if it is one.
    ///
    /// ```
    /// use emberhut_core::secret_link::SecretName;
    ///
    /// assert!(SecretName::parse("slack-token").is_ok());
    /// assert!(SecretName::parse("../etc/passwd").is_err());
    /// assert!(SecretName::parse(".hidden").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`InvalidSecretName`].
    pub fn parse(name: &str) -> Result<Self, InvalidSecretName> {
        let fits = !name.is_empty()
            && name.len() <= NAME_LIMIT
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if fits {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidSecretName(name.to_owned()))
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The most secrets one link asks for.
pub const MAX_NAMES: usize = 6;

/// Names a link may never ask for: the server's own key file's name.
const RESERVED: &[&str] = &["api-key"];

/// Why the names a link was asked for can't be its names.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidLinkRequest {
    /// None, or more than [`MAX_NAMES`].
    #[error("a link asks for 1 to {MAX_NAMES} secrets")]
    Count,
    /// A name that is not lowercase letters, digits and dashes.
    #[error("names.{0}: lowercase letters, digits and dashes only (slack-user-token)")]
    Name(usize),
    /// A name that is reserved.
    #[error("names.{0}: that name is reserved")]
    Reserved(usize),
}

/// The names a link asks for: 1 to [`MAX_NAMES`] of them, each lowercase ASCII letters,
/// digits and dashes (`slack-user-token`), and none reserved. They are file names in the
/// server's secrets directory, so this is stricter than [`SecretName`].
///
/// ```
/// use emberhut_core::secret_link::{InvalidLinkRequest, link_names};
///
/// assert_eq!(link_names(&["slack-token"]).unwrap()[0].as_str(), "slack-token");
/// assert_eq!(link_names(&["ok", "Not-OK"]), Err(InvalidLinkRequest::Name(1)));
/// ```
///
/// # Errors
///
/// [`InvalidLinkRequest`].
pub fn link_names<S: AsRef<str>>(names: &[S]) -> Result<Vec<SecretName>, InvalidLinkRequest> {
    if names.is_empty() || names.len() > MAX_NAMES {
        return Err(InvalidLinkRequest::Count);
    }
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let name = name.as_ref();
            let plain = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !plain {
                return Err(InvalidLinkRequest::Name(index));
            }
            if RESERVED.contains(&name) {
                return Err(InvalidLinkRequest::Reserved(index));
            }
            SecretName::parse(name).map_err(|_| InvalidLinkRequest::Name(index))
        })
        .collect()
}

/// A stored link, as found by its token's hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretLinkRecord {
    /// Its row.
    pub id: SecretLinkId,
    /// The secrets its page asks for, in order.
    pub names: Vec<SecretName>,
    /// Why they are wanted, as the page says it.
    pub why: String,
    /// When it stops working.
    pub expires_at: Timestamp,
}

impl SecretLinkRecord {
    /// Whether the link still works at `now`.
    #[must_use]
    pub fn works_at(&self, now: Timestamp) -> bool {
        now <= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::OsRandom;

    #[test]
    fn a_token_is_32_url_safe_characters_and_looks_like_one() {
        let token = LinkToken::generate(&OsRandom).unwrap();
        assert_eq!(token.as_str().len(), 32);
        assert!(looks_like_token(token.as_str()));
        assert_eq!(
            token.hash().as_bytes(),
            SecretHash::of(token.as_str()).as_bytes()
        );
        assert!(!format!("{token:?}").contains(token.as_str()));
    }

    #[test]
    fn only_long_url_safe_text_is_looked_up() {
        assert!(looks_like_token("abcdefghij_klmnopq-rs"));
        assert!(!looks_like_token("abcdefghijklmnopqrs"));
        assert!(!looks_like_token("abcdefghijklmnopqrstu.v"));
        assert!(!looks_like_token("abcdefghijklmnopqrstu%2F"));
    }

    #[test]
    fn secret_names_are_plain_file_names() {
        for name in ["a", "slack-token", "apns-key.p8", "A_1"] {
            assert!(SecretName::parse(name).is_ok(), "{name}");
        }
        for name in ["", ".env", "a/b", "a\\b", "..", "a b", "é", &"x".repeat(65)] {
            assert!(SecretName::parse(name).is_err(), "{name}");
        }
        assert_eq!(SecretName::parse("x").unwrap().to_string(), "x");
    }

    #[test]
    fn a_link_asks_for_one_to_six_plain_names() {
        let names = link_names(&["a", "slack-user-token", "key-2"]).unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(link_names::<&str>(&[]), Err(InvalidLinkRequest::Count));
        assert_eq!(link_names(&["a"; 7]), Err(InvalidLinkRequest::Count));
        assert!(link_names(&["a"; 6]).is_ok());
        for bad in ["", "A", "a_b", "a.b", "a/b", "é", "a b"] {
            assert_eq!(
                link_names(&["ok", bad]),
                Err(InvalidLinkRequest::Name(1)),
                "{bad}"
            );
        }
        assert_eq!(
            link_names(&["api-key"]),
            Err(InvalidLinkRequest::Reserved(0))
        );
        assert_eq!(
            link_names(&["x".repeat(65)]),
            Err(InvalidLinkRequest::Name(0)),
            "too long for a file name"
        );
    }

    #[test]
    fn a_link_works_until_it_expires() {
        let at = Timestamp::from_second(1_800_000_000).unwrap();
        let record = SecretLinkRecord {
            id: SecretLinkId::new(1),
            names: vec![SecretName::parse("x").unwrap()],
            why: "because".to_owned(),
            expires_at: at,
        };
        assert!(record.works_at(at));
        assert!(!record.works_at(at + SignedDuration::from_millis(1)));
    }
}
