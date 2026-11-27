//! The bodies of plain acknowledgements and of refusals.

use serde::{Deserialize, Serialize};

use crate::serde_util::is_false;

/// What the server sends when it refuses a request or fails: `{ "error": "..." }`, with
/// `"ok": false` on some routes and `"signIn": true` when the request needs a signed-in device.
///
/// Status codes that carry it: 400 (a bad request), 401 (not let in, or `signIn`), 404
/// (`"not found"`), 405, 429 and 500 (the server's own failure, with its message).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    /// `false` on the routes that answer `{ ok, ... }`; left out on the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// What went wrong, in words meant for a person.
    pub error: String,
    /// The server enforces sign-in and this request had no valid session: sign in (or pair)
    /// first. Only on a 401.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sign_in: bool,
}

impl ErrorBody {
    /// The body the server sends when a request needs a session: 401 with `signIn: true`.
    #[must_use]
    pub fn sign_in_required() -> Self {
        Self {
            ok: None,
            error: "Sign in first.".to_owned(),
            sign_in: true,
        }
    }

    /// A refusal with this message and no `ok` field.
    #[must_use]
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            ok: None,
            error: error.into(),
            sign_in: false,
        }
    }
}

/// `{ "ok": true }`: the request was done. Some routes add fields of their own; this type reads
/// only `ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OkReply {
    /// `true` when it was done.
    pub ok: bool,
}

impl OkReply {
    /// `{ "ok": true }`.
    pub const OK: Self = Self { ok: true };
}

impl Default for OkReply {
    fn default() -> Self {
        Self::OK
    }
}
