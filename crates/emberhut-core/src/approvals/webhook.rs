//! Webhook signatures: how the server and an approver's own app prove their calls to each
//! other, with the approver's secret.
//!
//! A call carries `x-emberhut-timestamp` (unix seconds) and `x-emberhut-signature: v1=<hex>`, the
//! HMAC-SHA256 over `"<ts>.<METHOD>.<path>."` followed by the body's bytes. A call signed more
//! than five minutes from the receiver's clock is refused, so a recorded call can't be replayed
//! later; the method and path are covered, so a signature for one route is not one for another.

use jiff::Timestamp;

use super::mac::{SignatureError, check_hex, fresh, hmac, to_hex};

/// The header carrying the timestamp.
pub const TIMESTAMP_HEADER: &str = "x-emberhut-timestamp";
/// The header carrying the signature.
pub const SIGNATURE_HEADER: &str = "x-emberhut-signature";
/// The signature scheme's version.
const SCHEME: &str = "v1";

/// The `x-emberhut-signature` value for a call made at `timestamp` (unix seconds).
#[must_use]
pub fn sign(secret: &[u8], timestamp: i64, method: &str, path: &str, body: &[u8]) -> String {
    let head = format!("{timestamp}.{method}.{path}.");
    to_hex(hmac(secret, &[head.as_bytes(), body]), SCHEME)
}

/// Checks a call's `timestamp` and `signature` headers against `secret`, at `now`.
///
/// ```
/// use jiff::Timestamp;
/// use emberhut_core::approvals::webhook::{sign, verify};
///
/// let now = Timestamp::from_second(1_800_000_000).unwrap();
/// let signature = sign(b"secret", 1_800_000_000, "POST", "/api/approvals/7/webhook", b"{}");
/// assert!(verify(b"secret", "1800000000", &signature, "POST", "/api/approvals/7/webhook", b"{}", now).is_ok());
/// assert!(verify(b"secret", "1800000000", &signature, "POST", "/api/approvals/8/webhook", b"{}", now).is_err());
/// ```
///
/// # Errors
///
/// [`SignatureError`].
pub fn verify(
    secret: &[u8],
    timestamp: &str,
    signature: &str,
    method: &str,
    path: &str,
    body: &[u8],
    now: Timestamp,
) -> Result<(), SignatureError> {
    let at = fresh(timestamp, now)?;
    let head = format!("{at}.{method}.{path}.");
    check_hex(hmac(secret, &[head.as_bytes(), body]), SCHEME, signature)
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    const PATH: &str = "/api/approvals/7/webhook";
    const BODY: &[u8] = br#"{"decision":"approve"}"#;

    fn now() -> Timestamp {
        Timestamp::from_second(1_800_000_000).unwrap()
    }

    fn signed() -> String {
        sign(b"secret", 1_800_000_000, "POST", PATH, BODY)
    }

    #[test]
    fn a_signed_call_verifies() {
        assert!(signed().starts_with("v1="));
        assert_eq!(
            verify(
                b"secret",
                "1800000000",
                &signed(),
                "POST",
                PATH,
                BODY,
                now()
            ),
            Ok(())
        );
    }

    #[test]
    fn any_change_fails() {
        let checks = [
            verify(b"other", "1800000000", &signed(), "POST", PATH, BODY, now()),
            verify(
                b"secret",
                "1800000001",
                &signed(),
                "POST",
                PATH,
                BODY,
                now(),
            ),
            verify(b"secret", "1800000000", &signed(), "PUT", PATH, BODY, now()),
            verify(
                b"secret",
                "1800000000",
                &signed(),
                "POST",
                "/x",
                BODY,
                now(),
            ),
            verify(
                b"secret",
                "1800000000",
                &signed(),
                "POST",
                PATH,
                b"{}",
                now(),
            ),
        ];
        for check in checks {
            assert_eq!(check, Err(SignatureError::Mismatch));
        }
    }

    #[test]
    fn an_old_call_is_refused() {
        let later = now() + SignedDuration::from_mins(5) + SignedDuration::from_secs(1);
        assert_eq!(
            verify(
                b"secret",
                "1800000000",
                &signed(),
                "POST",
                PATH,
                BODY,
                later
            ),
            Err(SignatureError::Stale)
        );
    }

    #[test]
    fn a_malformed_header_is_refused() {
        assert_eq!(
            verify(
                b"secret",
                "1800000000",
                "sha256=00",
                "POST",
                PATH,
                BODY,
                now()
            ),
            Err(SignatureError::Malformed)
        );
    }
}
