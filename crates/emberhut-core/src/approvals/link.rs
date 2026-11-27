//! Signed links: a one-time page an approver opens without an account, `/approve/{token}`.
//!
//! The token carries the request's id, the link's expiry, a random nonce and the payload's
//! hash, with an HMAC-SHA256 over all of them keyed by the server's link key, in URL-safe
//! base64. [`verify`] checks the MAC (in constant time) and the expiry; the server then checks
//! the claims against the request ([`LinkClaims::is_for`]) and that the nonce's hash is still
//! the request's (it is cleared by the first decision, so a link works once). The server
//! stores only the nonce's hash ([`LinkNonce::hash`]).

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Mac as _;
use jiff::{SignedDuration, Timestamp};
use subtle::ConstantTimeEq as _;

use super::mac::hmac;
use super::payload::PayloadHash;
use crate::random::{self, RandomError, RandomSource};
use crate::{ApprovalId, SecretHash};

/// How long a link works: as long as the request waits for its answer.
pub const LINK_LIFETIME: SignedDuration = super::state::PENDING_LIFETIME;

/// What the MAC's input starts with, so a link's MAC is never another message's.
const DOMAIN: &[u8] = b"emberhut-approval-link-v1";
/// The bytes of a link key.
const KEY_LEN: usize = 32;
/// The bytes of a nonce.
const NONCE_LEN: usize = 16;
/// id (8) + expiry (8) + nonce + payload hash (32).
const CLAIMS_LEN: usize = 16 + NONCE_LEN + 32;
/// The claims and the MAC.
const TOKEN_LEN: usize = CLAIMS_LEN + 32;

/// The server's key for signed links (the `approval-link-key` secret). `Debug` never shows it.
#[derive(Clone)]
pub struct LinkKey([u8; KEY_LEN]);

/// A stored link key that can't be one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a link key is 32 bytes in URL-safe base64")]
pub struct InvalidLinkKey;

impl LinkKey {
    /// A new key from `random`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when `random` fails.
    pub fn generate(random: &dyn RandomSource) -> Result<Self, RandomError> {
        Ok(Self(random::bytes::<KEY_LEN>(random)?))
    }

    /// The key as its secret file holds it: URL-safe base64.
    #[must_use]
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    /// The key its secret file holds (surrounding whitespace ignored).
    ///
    /// # Errors
    ///
    /// [`InvalidLinkKey`].
    pub fn decode(text: &str) -> Result<Self, InvalidLinkKey> {
        URL_SAFE_NO_PAD
            .decode(text.trim())
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .map(Self)
            .ok_or(InvalidLinkKey)
    }
}

impl fmt::Debug for LinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LinkKey(..)")
    }
}

/// A link's random nonce. `Debug` never shows it.
#[derive(Clone, Copy)]
pub struct LinkNonce([u8; NONCE_LEN]);

impl LinkNonce {
    /// A new nonce from `random`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when `random` fails.
    pub fn generate(random: &dyn RandomSource) -> Result<Self, RandomError> {
        Ok(Self(random::bytes::<NONCE_LEN>(random)?))
    }

    /// What the database keeps of it, to look it up by.
    #[must_use]
    pub fn hash(&self) -> SecretHash {
        SecretHash::of_bytes(&self.0)
    }
}

impl fmt::Debug for LinkNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LinkNonce(..)")
    }
}

/// What a valid token says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkClaims {
    /// The request it is for.
    pub request: ApprovalId,
    /// The payload it shows and decides.
    pub payload_hash: PayloadHash,
    /// When it stops working.
    pub expires: Timestamp,
    /// Its nonce.
    pub nonce: LinkNonce,
}

impl LinkClaims {
    /// Whether the link is for `request` with this payload hash (compared in constant time).
    #[must_use]
    pub fn is_for(&self, request: ApprovalId, payload_hash: &PayloadHash) -> bool {
        self.request == request && self.payload_hash == *payload_hash
    }
}

/// Compared in constant time.
impl PartialEq for LinkNonce {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl Eq for LinkNonce {}

/// Why a token is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    /// Not a token at all.
    #[error("not an approval link")]
    Malformed,
    /// Its MAC is not the key's: changed, or made with another key.
    #[error("the approval link is not genuine")]
    Forged,
    /// Past its expiry.
    #[error("the approval link has expired")]
    Expired,
}

/// The token of a link for `request`, showing `payload_hash`, working until `expires`.
#[must_use]
pub fn link_token(
    key: &LinkKey,
    request: ApprovalId,
    payload_hash: &PayloadHash,
    expires: Timestamp,
    nonce: &LinkNonce,
) -> String {
    let mut bytes = Vec::with_capacity(TOKEN_LEN);
    bytes.extend_from_slice(&request.get().to_be_bytes());
    bytes.extend_from_slice(&expires.as_second().to_be_bytes());
    bytes.extend_from_slice(&nonce.0);
    bytes.extend_from_slice(payload_hash.as_bytes());
    let tag = hmac(&key.0, &[DOMAIN, &bytes]).finalize().into_bytes();
    bytes.extend_from_slice(&tag);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The claims of `token` if `key` signed it and it has not expired at `now`.
///
/// ```
/// use jiff::{SignedDuration, Timestamp};
/// use emberhut_core::approvals::link::{LinkError, LinkKey, LinkNonce, link_token, verify};
/// use emberhut_core::approvals::Sha256Digest;
/// use emberhut_core::{ApprovalId, OsRandom};
///
/// let key = LinkKey::generate(&OsRandom).unwrap();
/// let nonce = LinkNonce::generate(&OsRandom).unwrap();
/// let hash = Sha256Digest::of(b"payload");
/// let now = Timestamp::from_second(1_800_000_000).unwrap();
/// let expires = now + SignedDuration::from_hours(1);
/// let token = link_token(&key, ApprovalId::new(7), &hash, expires, &nonce);
/// let claims = verify(&key, &token, now).unwrap();
/// assert!(claims.is_for(ApprovalId::new(7), &hash));
/// assert_eq!(verify(&key, &token, expires), Err(LinkError::Expired));
/// ```
///
/// # Errors
///
/// [`LinkError`].
pub fn verify(key: &LinkKey, token: &str, now: Timestamp) -> Result<LinkClaims, LinkError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| LinkError::Malformed)?;
    if bytes.len() != TOKEN_LEN {
        return Err(LinkError::Malformed);
    }
    let (claims, tag) = bytes.split_at(CLAIMS_LEN);
    hmac(&key.0, &[DOMAIN, claims])
        .verify_slice(tag)
        .map_err(|_| LinkError::Forged)?;
    let (request, rest) = take::<8>(claims);
    let (expires, rest) = take::<8>(rest);
    let (nonce, rest) = take::<NONCE_LEN>(rest);
    let payload_hash = PayloadHash::from_bytes(rest).ok_or(LinkError::Malformed)?;
    let expires =
        Timestamp::from_second(i64::from_be_bytes(expires)).map_err(|_| LinkError::Malformed)?;
    if now >= expires {
        return Err(LinkError::Expired);
    }
    Ok(LinkClaims {
        request: ApprovalId::new(i64::from_be_bytes(request)),
        payload_hash,
        expires,
        nonce: LinkNonce(nonce),
    })
}

/// The first `N` bytes of `bytes` and the rest; zeros for bytes that aren't there (the caller
/// checked the length).
fn take<const N: usize>(bytes: &[u8]) -> ([u8; N], &[u8]) {
    let mut head = [0; N];
    let split = N.min(bytes.len());
    head[..split].copy_from_slice(&bytes[..split]);
    (head, &bytes[split..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OsRandom;
    use crate::approvals::Sha256Digest;

    struct Fixture {
        key: LinkKey,
        nonce: LinkNonce,
        hash: PayloadHash,
        now: Timestamp,
        expires: Timestamp,
    }

    fn fixture() -> Fixture {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        Fixture {
            key: LinkKey::generate(&OsRandom).unwrap(),
            nonce: LinkNonce::generate(&OsRandom).unwrap(),
            hash: Sha256Digest::of(b"payload a"),
            now,
            expires: now + LINK_LIFETIME,
        }
    }

    fn token_for(f: &Fixture, request: i64) -> String {
        link_token(
            &f.key,
            ApprovalId::new(request),
            &f.hash,
            f.expires,
            &f.nonce,
        )
    }

    #[test]
    fn a_token_verifies_with_its_claims() {
        let f = fixture();
        let claims = verify(&f.key, &token_for(&f, 7), f.now).unwrap();
        assert_eq!(claims.request, ApprovalId::new(7));
        assert_eq!(claims.payload_hash, f.hash);
        assert_eq!(claims.expires, f.expires);
        assert_eq!(claims.nonce, f.nonce);
    }

    #[test]
    fn a_token_for_a_is_not_for_b_or_another_payload() {
        let f = fixture();
        let claims = verify(&f.key, &token_for(&f, 7), f.now).unwrap();
        assert!(claims.is_for(ApprovalId::new(7), &f.hash));
        assert!(!claims.is_for(ApprovalId::new(8), &f.hash));
        assert!(!claims.is_for(ApprovalId::new(7), &Sha256Digest::of(b"payload b")));
    }

    #[test]
    fn an_expired_token_fails() {
        let f = fixture();
        let token = token_for(&f, 7);
        assert_eq!(verify(&f.key, &token, f.expires), Err(LinkError::Expired));
        assert!(verify(&f.key, &token, f.expires - SignedDuration::from_secs(1)).is_ok());
    }

    #[test]
    fn a_tampered_token_fails() {
        let f = fixture();
        let token = token_for(&f, 7);
        let mut bytes = URL_SAFE_NO_PAD.decode(&token).unwrap();
        // Point it at request 8, push its expiry on, swap its hash: each breaks the MAC.
        for at in [7, 15, 40, TOKEN_LEN - 1] {
            let mut changed = bytes.clone();
            changed[at] ^= 1;
            assert_eq!(
                verify(&f.key, &URL_SAFE_NO_PAD.encode(changed), f.now),
                Err(LinkError::Forged),
                "byte {at}"
            );
        }
        bytes.pop();
        assert_eq!(
            verify(&f.key, &URL_SAFE_NO_PAD.encode(bytes), f.now),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            verify(&f.key, "not a token!", f.now),
            Err(LinkError::Malformed)
        );
    }

    #[test]
    fn another_key_fails() {
        let f = fixture();
        let other = LinkKey::generate(&OsRandom).unwrap();
        assert_eq!(
            verify(&other, &token_for(&f, 7), f.now),
            Err(LinkError::Forged)
        );
    }

    #[test]
    fn keys_round_trip_and_hide() {
        let key = LinkKey::generate(&OsRandom).unwrap();
        let again = LinkKey::decode(&format!("{}\n", key.encode())).unwrap();
        assert_eq!(again.0, key.0);
        assert_eq!(format!("{key:?}"), "LinkKey(..)");
        assert!(matches!(LinkKey::decode("short"), Err(InvalidLinkKey)));
        let nonce = LinkNonce::generate(&OsRandom).unwrap();
        assert_eq!(format!("{nonce:?}"), "LinkNonce(..)");
    }

    #[test]
    fn tokens_are_url_safe() {
        let f = fixture();
        let token = token_for(&f, i64::MAX);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert!(crate::secret_link::looks_like_token(&token));
    }
}
