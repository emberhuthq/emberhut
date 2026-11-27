//! HMAC-SHA256, the one MAC every approval channel signs with, and the check of a signed
//! request's age.

use hmac::{Hmac, KeyInit as _, Mac as _};
use jiff::{SignedDuration, Timestamp};
use sha2::Sha256;

/// How old a signed request may be, and how far ahead of the server's clock.
pub const MAX_SIGNATURE_AGE: SignedDuration = SignedDuration::from_mins(5);

/// Why a signed request is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SignatureError {
    /// A timestamp or signature that can't be read.
    #[error("the signature or its timestamp is malformed")]
    Malformed,
    /// Signed more than five minutes from the server's clock, either way.
    #[error("the signature is too old, or from the future")]
    Stale,
    /// The signature is not the key's over this request.
    #[error("the signature does not match")]
    Mismatch,
}

/// HMAC-SHA256 keyed with `key`, over `parts` in order.
pub(super) fn hmac(key: &[u8], parts: &[&[u8]]) -> Hmac<Sha256> {
    let mut mac = keyed(key);
    for part in parts {
        mac.update(part);
    }
    mac
}

#[expect(
    clippy::expect_used,
    reason = "HMAC takes a key of any length: new_from_slice never fails for it"
)]
fn keyed(key: &[u8]) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length")
}

/// The unix-seconds timestamp `text`, refused when it is not one or is more than
/// [`MAX_SIGNATURE_AGE`] away from `now`.
pub(super) fn fresh(text: &str, now: Timestamp) -> Result<i64, SignatureError> {
    let seconds: i64 = text.parse().map_err(|_| SignatureError::Malformed)?;
    let at = Timestamp::from_second(seconds).map_err(|_| SignatureError::Malformed)?;
    if now.duration_since(at).abs() > MAX_SIGNATURE_AGE {
        return Err(SignatureError::Stale);
    }
    Ok(seconds)
}

/// Checks `signature`, a header value `<scheme>=<hex>`, against `mac`, in constant time.
pub(super) fn check_hex(
    mac: Hmac<Sha256>,
    scheme: &str,
    signature: &str,
) -> Result<(), SignatureError> {
    let hex = signature
        .strip_prefix(scheme)
        .and_then(|rest| rest.strip_prefix('='))
        .ok_or(SignatureError::Malformed)?;
    let bytes = data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.as_bytes())
        .map_err(|_| SignatureError::Malformed)?;
    mac.verify_slice(&bytes)
        .map_err(|_| SignatureError::Mismatch)
}

/// `mac`'s tag, as `<scheme>=<lower-case hex>`.
pub(super) fn to_hex(mac: Hmac<Sha256>, scheme: &str) -> String {
    format!(
        "{scheme}={}",
        data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_4231_case_2() {
        let mac = hmac(b"Jefe", &[b"what do ya want ", b"for nothing?"]);
        assert_eq!(
            to_hex(mac, "v"),
            "v=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn freshness() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        assert_eq!(fresh("1800000000", now), Ok(1_800_000_000));
        assert_eq!(fresh("1799999700", now), Ok(1_799_999_700));
        assert_eq!(fresh("1799999699", now), Err(SignatureError::Stale));
        assert_eq!(fresh("1800000301", now), Err(SignatureError::Stale));
        assert_eq!(fresh("soon", now), Err(SignatureError::Malformed));
        assert_eq!(fresh("", now), Err(SignatureError::Malformed));
    }

    #[test]
    fn hex_checks() {
        let tag = to_hex(hmac(b"k", &[b"m"]), "v1");
        assert_eq!(check_hex(hmac(b"k", &[b"m"]), "v1", &tag), Ok(()));
        assert_eq!(
            check_hex(hmac(b"k", &[b"n"]), "v1", &tag),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            check_hex(hmac(b"k", &[b"m"]), "v0", &tag),
            Err(SignatureError::Malformed)
        );
        assert_eq!(
            check_hex(hmac(b"k", &[b"m"]), "v1", "v1=zz"),
            Err(SignatureError::Malformed)
        );
        assert_eq!(
            check_hex(hmac(b"k", &[b"m"]), "v1", "v1=00"),
            Err(SignatureError::Mismatch)
        );
    }
}
