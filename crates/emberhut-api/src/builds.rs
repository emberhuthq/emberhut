//! The newest build of the app per platform: the state's `builds`, and the release step's
//! report of a build a store has ready, `POST /api/settings` with
//! `{ "build": { "platform": "ios" | "mac", "number": 12 } }`.
//!
//! An app older than its platform's build offers the update ("Build N is ready"). A report only
//! ever moves a platform's number up. It is [`crate::settings::SettingsRequest`]'s `build`; a
//! refused report is 400 with `{ ok: false, error }`, a taken one answers like every
//! `/api/settings` change ([`crate::settings::SettingsReply`]).

use serde::{Deserialize, Serialize};

/// The newest build reported per platform: `null` where none was. The server writes
/// both fields always.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LatestBuilds {
    /// The phone app's newest build.
    #[serde(default)]
    pub ios: Option<u32>,
    /// The desktop app's newest build.
    #[serde(default)]
    pub mac: Option<u32>,
}

/// A platform builds are reported for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum BuildPlatform {
    /// The phone app.
    Ios,
    /// The desktop app.
    Mac,
}

/// Build `number` is ready on `platform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BuildReport {
    /// Where.
    pub platform: BuildPlatform,
    /// Which build: a whole number from 1.
    #[cfg_attr(feature = "openapi", schema(minimum = 1, maximum = 2_147_483_647))]
    pub number: u32,
}
