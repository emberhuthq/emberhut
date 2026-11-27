//! Slack's request signatures: how a button click on an approval DM proves it came from Slack.
//!
//! Slack signs each request with the workspace's signing secret: `X-Slack-Signature: v0=<hex>`,
//! the HMAC-SHA256 over `v0:<X-Slack-Request-Timestamp>:<body>`. A request more than five
//! minutes from the server's clock is refused, as Slack advises, so it can't be replayed.

use jiff::Timestamp;

use super::mac::{SignatureError, check_hex, fresh, hmac};

/// The header carrying the timestamp.
pub const TIMESTAMP_HEADER: &str = "x-slack-request-timestamp";
/// The header carrying the signature.
pub const SIGNATURE_HEADER: &str = "x-slack-signature";
/// The signature scheme's version.
const SCHEME: &str = "v0";

/// The name of the secret holding a Slack workspace's signing secret: `slack-<team>-signing`,
/// the team id in lower case.
#[must_use]
pub fn signing_secret_name(team: &str) -> String {
    format!("slack-{}-signing", team.to_ascii_lowercase())
}

/// Checks a Slack request's `timestamp` and `signature` headers over its raw `body`, at `now`.
///
/// # Errors
///
/// [`SignatureError`].
pub fn verify(
    signing_secret: &[u8],
    timestamp: &str,
    signature: &str,
    body: &[u8],
    now: Timestamp,
) -> Result<(), SignatureError> {
    let at = fresh(timestamp, now)?;
    let head = format!("{SCHEME}:{at}:");
    check_hex(
        hmac(signing_secret, &[head.as_bytes(), body]),
        SCHEME,
        signature,
    )
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    /// Slack's own example of a signed request (its "Verifying requests from Slack" guide).
    const SECRET: &[u8] = b"8f742231b10e8888abcd99yyyzzz85a5";
    const TIMESTAMP: &str = "1531420618";
    const BODY: &[u8] = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8PSS9T3V&channel_name=foobar&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c447fce8b6703c";
    const SIGNATURE: &str = "v0=a2114d57b48eac39b9ad189dd8316235a7b4a8d21a10bd27519666489c69b503";

    fn now() -> Timestamp {
        Timestamp::from_second(1_531_420_618).unwrap()
    }

    #[test]
    fn slacks_example_verifies() {
        assert_eq!(verify(SECRET, TIMESTAMP, SIGNATURE, BODY, now()), Ok(()));
    }

    #[test]
    fn a_changed_body_or_secret_fails() {
        assert_eq!(
            verify(SECRET, TIMESTAMP, SIGNATURE, b"token=other", now()),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify(b"another secret", TIMESTAMP, SIGNATURE, BODY, now()),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn an_old_request_is_refused() {
        let later = now() + SignedDuration::from_mins(6);
        assert_eq!(
            verify(SECRET, TIMESTAMP, SIGNATURE, BODY, later),
            Err(SignatureError::Stale)
        );
    }

    #[test]
    fn the_signing_secrets_name() {
        assert_eq!(signing_secret_name("T0EXAMPLE"), "slack-t0example-signing");
    }
}
