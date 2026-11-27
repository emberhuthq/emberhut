//! Header names and values the contract uses, beyond plain JSON bodies.
//!
//! Header names are given in lower case, as HTTP/2 writes them; HTTP/1.1 compares them without
//! regard to case.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::auth::AccessToken;

/// `Authorization: Bearer <access token>` on every request of a signed-in device.
pub const AUTHORIZATION: &str = "authorization";

/// The state's fingerprint, on a `GET /api/state` answer (200 and 304).
pub const ETAG: &str = "etag";

/// The fingerprint of the state the client holds; the server answers 304 with no body when the
/// state is still that one.
pub const IF_NONE_MATCH: &str = "if-none-match";

/// Sent with `GET /api/state` while the room is on screen in front of the person: its value is
/// the room id. For the next 10 seconds the server doesn't push that room's replies to the
/// phone, since they are being read already. Its name is in [`crate::wire`].
pub const LOOKING: &str = crate::wire::LOOKING_HEADER;

/// On a relayed connection only: the machine's verdict on the request's `Authorization`,
/// [`EmberhutAuth`]. The relay reads and removes it; a direct client never sees it.
pub const EMBERHUT_AUTH: &str = "emberhut-auth";

/// On every answer the relay makes itself: a
/// [`RelayError`](crate::relay::RelayError) code. The relay removes it from the machine's
/// answers, so clients act on it alone.
pub const EMBERHUT_RELAY_ERROR: &str = "emberhut-relay-error";

/// The value of [`EMBERHUT_AUTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmberhutAuth {
    /// `ok`: the access token verified.
    Ok,
    /// `refused`: it didn't.
    Refused,
}

/// A [`EMBERHUT_AUTH`] value other than `ok` and `refused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("Emberhut-Auth is ok or refused")]
pub struct EmberhutAuthError;

impl EmberhutAuth {
    /// The header value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Refused => "refused",
        }
    }
}

impl FromStr for EmberhutAuth {
    type Err = EmberhutAuthError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ok" => Ok(Self::Ok),
            "refused" => Ok(Self::Refused),
            _ => Err(EmberhutAuthError),
        }
    }
}

impl fmt::Display for EmberhutAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `Authorization` value for an access token: `Bearer <token>`.
#[must_use]
pub fn bearer(token: &AccessToken) -> String {
    format!("Bearer {}", token.expose_secret())
}

/// An entity tag as the server sent it, quotes included (`"q1w2e3"`): compare it, send it back
/// in `If-None-Match`, never look inside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ETag(String);

impl ETag {
    /// Wraps a header value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The header value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_value() {
        assert_eq!(bearer(&AccessToken::new("abc.def")), "Bearer abc.def");
    }

    #[test]
    fn emberhut_auth_values() {
        assert_eq!(EmberhutAuth::Ok.to_string().parse(), Ok(EmberhutAuth::Ok));
        assert_eq!(
            EmberhutAuth::Refused.to_string().parse(),
            Ok(EmberhutAuth::Refused)
        );
        assert_eq!("OK".parse::<EmberhutAuth>(), Err(EmberhutAuthError));
    }

    #[test]
    fn etag_is_opaque_text() {
        let tag = ETag::new("\"q1w2\"");
        assert_eq!(tag.as_str(), "\"q1w2\"");
        assert_eq!(tag.to_string(), "\"q1w2\"");
    }
}
