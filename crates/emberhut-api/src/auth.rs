//! Signing a device in: pairing with a one-time code, renewing the access token, and what the
//! state says about sign-in.
//!
//! A device's session is two tokens. The access token goes with every request
//! (`Authorization: Bearer ...`) and lasts an hour. The refresh token buys a new access token at
//! [`crate::paths::REFRESH`] and turns over on every use: the old one never works again, so two
//! renewals must never run at once with the same token.
//!
//! The server runs in one of two modes ([`AuthMode`]). In `report` a request without a session
//! is let through (and noted); in `enforce` it is refused with 401 and
//! [`crate::error::ErrorBody::sign_in`] set.

use std::fmt;

use serde::{Deserialize, Serialize};

use emberhut_identity::PublicKey;

use crate::connect::MachineIdentity;
use crate::ids::{DeviceId, InstallId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

/// What a redacting `Debug` shows in place of a secret: `<redacted>`.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Defines a token newtype whose `Debug` never shows the token.
macro_rules! secret_token {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        pub struct $name(String);

        impl $name {
            /// Wraps a token.
            #[must_use]
            pub fn new(token: impl Into<String>) -> Self {
                Self(token.into())
            }

            /// The token itself, to send. Never log it.
            #[must_use]
            pub fn expose_secret(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}

secret_token! {
    /// The short-lived token sent with every request (`Authorization: Bearer ...`).
    AccessToken
}

secret_token! {
    /// The token that renews the access token. It turns over on every use.
    RefreshToken
}

/// `POST /api/auth/pair`: a one-time code (made on a signed-in device) for a session of this
/// device's own.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct PairRequest {
    /// The code as typed: case, spaces and the dash don't matter (`abcd efgh` is `ABCD-EFGH`).
    /// Not blank, at most 32 characters (a code is 8, shown as 9 with its dash).
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 32))
    )]
    pub code: String,
    /// This installation: the same install pairing again keeps its device row. Left out, or
    /// empty, the device gets a new row each time it pairs .
    #[serde(
        default,
        deserialize_with = "crate::serde_util::empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub install: Option<InstallId>,
    /// The device's name in the device list; left out, null or empty, "A device". At most 60
    /// UTF-16 units; longer is refused.
    #[serde(default, deserialize_with = "crate::serde_util::null_as_default")]
    #[cfg_attr(feature = "validate", garde(length(utf16, max = 60)))]
    pub name: String,
    /// The kind of device (`macOS`, `linux`, `Pixel 8`); may be left out or null. At most 40
    /// UTF-16 units.
    #[serde(default, deserialize_with = "crate::serde_util::null_as_default")]
    #[cfg_attr(feature = "validate", garde(length(utf16, max = 40)))]
    pub model: String,
    /// The device's own Ed25519 public key, base64url without padding, which the machine keeps
    /// on the device's row. Clients that predate it leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub public_key: Option<PublicKey>,
}

// Its `Debug` leaves the code out: a one-time code is a password until it is used. (A plain
// comment, not a doc comment: the schema's description is the type's doc comment.)
impl fmt::Debug for PairRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairRequest")
            .field("code", &Redacted)
            .field("install", &self.install)
            .field("name", &self.name)
            .field("model", &self.model)
            .field("public_key", &self.public_key)
            .finish()
    }
}

/// `POST /api/auth/refresh`: a new access token for this device's refresh token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct RefreshRequest {
    /// The refresh token the last grant gave. Not blank.
    #[cfg_attr(feature = "validate", garde(custom(|token: &RefreshToken, ctx: &()| crate::rules::not_blank(token.expose_secret(), ctx))))]
    pub refresh: RefreshToken,
    /// This installation; a device signed in before it sent one gets it recorded here. Left
    /// out or empty, none (as when pairing).
    #[serde(
        default,
        deserialize_with = "crate::serde_util::empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub install: Option<InstallId>,
}

/// What `POST /api/auth/pair` and `POST /api/auth/refresh` answer, accepted or refused, with
/// the HTTP status repeated in the body.
///
/// Accepted (200): `{ ok: true, status: 200, access, refresh, device, expires }`. Refused:
/// `{ ok: false, status, error }`, with status 400 (no code), 401 (a wrong, used or expired code;
/// a refresh token that is no longer valid: sign in again) or 429 (too many wrong codes).
/// The fields of the other shape are left out. `AuthReply::into_grant` splits the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthReply {
    /// Whether the device is signed in.
    pub ok: bool,
    /// The HTTP status of this answer.
    pub status: u16,
    /// Why it was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The new access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<AccessToken>,
    /// The new refresh token (the one sent no longer works).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshToken>,
    /// The device's row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceId>,
    /// When the access token runs out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Timestamp>,
    /// On an accepted pairing from an emberhut server: the machine's id and public key, for the
    /// device to pin. A server that predates it (a legacy host) leaves it out; a device that
    /// paired from a link, or already holds a key for the host, refuses an answer without it
    /// or with another id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<MachineIdentity>,
}

/// An accepted sign-in: the session a device keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGrant {
    /// The access token.
    pub access: AccessToken,
    /// The refresh token, for the next renewal.
    pub refresh: RefreshToken,
    /// The device's row, when the server named it.
    pub device: Option<DeviceId>,
    /// When the access token runs out, when the server said.
    pub expires: Option<Timestamp>,
}

/// A refused sign-in or renewal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRefusal {
    /// The HTTP status.
    pub status: u16,
    /// Why, in words meant for a person.
    pub error: String,
}

impl fmt::Display for AuthRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.error, self.status)
    }
}

impl std::error::Error for AuthRefusal {}

impl AuthReply {
    /// The 200 answer for a session.
    #[must_use]
    pub fn accepted(grant: SessionGrant) -> Self {
        Self {
            ok: true,
            status: 200,
            error: None,
            access: Some(grant.access),
            refresh: Some(grant.refresh),
            device: grant.device,
            expires: grant.expires,
            machine: None,
        }
    }

    /// A refusal with this status and reason.
    #[must_use]
    pub fn refused(status: u16, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            status,
            error: Some(error.into()),
            access: None,
            refresh: None,
            device: None,
            expires: None,
            machine: None,
        }
    }

    /// The session this answer grants, or why it doesn't.
    ///
    /// # Errors
    ///
    /// An [`AuthRefusal`] when `ok` is false, or when an accepted answer lacks a token (then
    /// with this answer's status and a reason saying so).
    pub fn into_grant(self) -> Result<SessionGrant, AuthRefusal> {
        match (self.ok, self.access, self.refresh) {
            (true, Some(access), Some(refresh)) => Ok(SessionGrant {
                access,
                refresh,
                device: self.device,
                expires: self.expires,
            }),
            (true, ..) => Err(AuthRefusal {
                status: self.status,
                error: "the server accepted the sign-in but sent no tokens".to_owned(),
            }),
            (false, ..) => Err(AuthRefusal {
                status: self.status,
                error: self.error.unwrap_or_else(|| "Not accepted.".to_owned()),
            }),
        }
    }
}

/// `POST /api/auth/pair/new`'s answer: a code for a new device, good once for 10 minutes.
///
/// The code is 8 characters from an alphabet without look-alikes (no 0/O, 1/I/L), shown as
/// `ABCD-EFGH`. A request that isn't from a signed-in device or the server's key is refused
/// with 401 and an `ErrorBody` (`ok: false`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PairingCode {
    /// Always `true`.
    pub ok: bool,
    /// The code as shown to the person, `ABCD-EFGH`.
    pub code: String,
    /// When it stops working.
    pub expires: Timestamp,
    /// The pairing link for the code (`https://emberhut.com/pair#id=…&code=…&route=…`), to show as
    /// text and as a QR code, from a server that has a machine id. A server that predates it
    /// (or a legacy host) leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

// Its `Debug` leaves the code out, as `PairRequest`'s does.
impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingCode")
            .field("ok", &self.ok)
            .field("code", &Redacted)
            .field("expires", &self.expires)
            .field("link", &self.link.as_ref().map(|_| Redacted))
            .finish()
    }
}

open_enum! {
    /// Whether the server refuses requests without a session.
    pub enum AuthMode {
        /// Requests without a session pass and are noted: what enforcing would refuse.
        Report = "report",
        /// Requests without a session are refused with 401 `signIn`.
        Enforce = "enforce",
    }
}

/// The state's `auth`: the sign-in mode and the signed-in devices (no tokens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthInfo {
    /// Report or enforce.
    pub mode: AuthMode,
    /// Whether an identity is bound (the first sign-in binds it).
    pub bound: bool,
    /// The signed-in devices, the most recently seen first.
    #[serde(default)]
    pub devices: Vec<Device>,
}

/// `GET /api/auth/status`: the state's [`AuthInfo`], and whether this request's session is a
/// signed-in device's. Asked with or without a session, in either mode; the device list is
/// sent only to a signed-in device, and is empty otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    /// The mode, whether the server is in use, and (for a signed-in device) the devices.
    #[serde(flatten)]
    pub info: AuthInfo,
    /// The request came with a signed-in device's current access token.
    pub signed_in: bool,
}

/// A signed-in device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Device {
    /// Its row: compare with this device's own (from its sign-in) to mark "this one".
    pub id: DeviceId,
    /// Its name.
    pub name: Option<String>,
    /// Its kind.
    pub model: Option<String>,
    /// When it first signed in.
    pub created: Option<Timestamp>,
    /// When it was last seen.
    pub seen: Option<Timestamp>,
}

/// `POST /api/settings` with `{ authRevoke }`: signs a device out. Its refresh token stops
/// working, so its next renewal is refused and it has to sign in again. Answered with
/// [`crate::error::OkReply`] (the state spread in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct RevokeRequest {
    /// The device to sign out (a row of [`AuthInfo::devices`]).
    #[cfg_attr(feature = "validate", garde(dive))]
    pub auth_revoke: DeviceId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_codes_never_show_in_debug() {
        let request = PairRequest {
            code: "WXYZ-2345".into(),
            install: Some(InstallId::new("install-1")),
            name: "Example".into(),
            model: "linux".into(),
            public_key: None,
        };
        let shown = format!("{request:?}");
        assert!(!shown.contains("WXYZ"), "{shown}");
        assert!(
            shown.contains("<redacted>") && shown.contains("Example"),
            "{shown}"
        );
        let made = PairingCode {
            ok: true,
            code: "WXYZ-2345".into(),
            expires: Timestamp::new("2026-01-01T00:10:00Z"),
            link: Some("https://emberhut.com/pair#id=x&code=WXYZ-2345".into()),
        };
        let shown = format!("{made:?}");
        assert!(!shown.contains("WXYZ"), "{shown}");
        assert!(shown.contains("2026-01-01T00:10:00Z"), "{shown}");
    }

    #[test]
    fn tokens_never_show_in_debug() {
        let reply = AuthReply::accepted(SessionGrant {
            access: AccessToken::new("access-secret"),
            refresh: RefreshToken::new("refresh-secret"),
            device: Some(DeviceId::new(3)),
            expires: None,
        });
        let shown = format!("{reply:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("AccessToken(<redacted>)"));
        assert!(shown.contains("RefreshToken(<redacted>)"));
    }

    #[test]
    fn a_refusal_is_an_error() {
        let refusal = AuthReply::refused(401, "Sign in again.")
            .into_grant()
            .unwrap_err();
        assert_eq!(
            refusal,
            AuthRefusal {
                status: 401,
                error: "Sign in again.".to_owned()
            }
        );
        assert_eq!(refusal.to_string(), "Sign in again. (401)");
    }

    #[test]
    fn an_accepted_reply_without_tokens_is_refused() {
        let mut reply = AuthReply::refused(200, "x");
        reply.ok = true;
        assert_eq!(reply.into_grant().unwrap_err().status, 200);
        let mut silent = AuthReply::refused(401, "x");
        silent.error = None;
        assert_eq!(silent.into_grant().unwrap_err().error, "Not accepted.");
    }
}
