//! `POST /api/devices`: where the server pushes a phone's alerts.
//!
//! An iPhone sends its APNs device token and whether it is a development build (`sandbox`) or
//! a release (`production`); an Android phone sends its FCM registration token with
//! `platform: "android"`. The answer is `{ "ok": true }`
//! ([`OkReply`](crate::error::OkReply)), or 400 with an [`ErrorBody`](crate::error::ErrorBody)
//! whose `error` says the token is not one (`"not a device token"`,
//! `"not an FCM registration token"`). Registering the same token again updates it.
//!
//! The state's `push` says whether the server can push at all (it has the keys of at least one
//! push service).

use serde::{Deserialize, Serialize};

/// The Android notification channel the server's FCM alerts name, which the Android app
/// creates ([`crate::wire`]).
pub const ANDROID_CHANNEL: &str = crate::wire::ANDROID_CHANNEL;

/// The `platform` an Android phone sends; anything else (or nothing) is an iPhone.
pub const ANDROID: &str = "android";

/// `POST /api/devices`: a phone's push address.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct PushRegistration {
    /// The push token: an iPhone's APNs device token (64 to 200 hex digits), or an Android
    /// phone's FCM registration token (32 to 4096 letters, digits, `_`, `:` or `-`). Left out,
    /// it is no token, and refused as one.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub token: String,
    /// `"android"` for an FCM token; left out for an iPhone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub platform: Option<String>,
    /// An iPhone's APNs service: `"production"` for a release build; anything else, or
    /// nothing, is the sandbox (a development build). Not read for Android.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub env: Option<String>,
}

impl PushRegistration {
    /// Whether this is an Android phone's FCM token.
    #[must_use]
    pub fn is_android(&self) -> bool {
        self.platform.as_deref() == Some(ANDROID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_android_is_android() {
        let mut registration = PushRegistration {
            token: "x".into(),
            ..PushRegistration::default()
        };
        assert!(!registration.is_android());
        registration.platform = Some("ios".into());
        assert!(!registration.is_android());
        registration.platform = Some(ANDROID.into());
        assert!(registration.is_android());
    }
}
