//! `GET /api/mac`: the owner's Mac as the app's Mac row shows it (online, locked, idle), from the
//! Mac helper on it; `POST /api/mac/lock` locks its screen ("Lock now"). Nothing unlocks it.
//!
//! [`helper`] and [`ExtensionFolder`] are the helper's own side of one more job: keeping the
//! browser extension's folder on the Mac in step with the zip this server serves
//! (`docs/mac-helper.md`).

use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Serialize, Serializer};

/// The Mac's row: whether its helper answers, and if so whether the screen is locked and how
/// long the Mac has been idle.
///
/// Written as `{ "online": false }` alone when the helper doesn't
/// answer, and `{ "online": true, "locked": …, "idle": … }` when it does, with `idle` `null` when
/// the helper doesn't say.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MacStatus {
    /// The Mac helper answered.
    pub online: bool,
    /// The screen is locked; only when `online`.
    #[serde(default)]
    pub locked: Option<bool>,
    /// Seconds since the last mouse or key input on the Mac; only when `online`, and `null` when
    /// the helper doesn't say.
    #[serde(default)]
    pub idle: Option<f64>,
}

impl MacStatus {
    /// The helper did not answer: `{ "online": false }`.
    pub const OFFLINE: Self = Self {
        online: false,
        locked: None,
        idle: None,
    };
}

impl Serialize for MacStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.online {
            let mut row = serializer.serialize_struct("MacStatus", 3)?;
            row.serialize_field("online", &true)?;
            row.serialize_field("locked", &self.locked.unwrap_or(false))?;
            row.serialize_field("idle", &self.idle)?;
            row.end()
        } else {
            let mut row = serializer.serialize_struct("MacStatus", 1)?;
            row.serialize_field("online", &false)?;
            row.end()
        }
    }
}

/// The Mac helper's routes for the browser extension's folder (the server calls them; the helper
/// serves them, each with its bearer token).
pub mod helper {
    /// `GET`: the extension unpacked in the folder ([`super::ExtensionFolder`]).
    pub const EXTENSION: &str = "/extension";
    /// `POST`, the zip as the body (`application/zip`): unpacks it into the folder, replacing what
    /// was there; 200 [`super::ExtensionFolder`], or 400 `{ "error" }`.
    pub const EXTENSION_INSTALL: &str = "/extension/install";
    /// The biggest zip `EXTENSION_INSTALL` takes: 20 MB.
    pub const EXTENSION_ZIP_MAX: usize = 20_000_000;
}

/// The browser extension's folder on the Mac: the version unpacked there (none when the folder
/// is empty or holds no readable manifest) and the folder's path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionFolder {
    /// The manifest's `version`.
    #[serde(default)]
    pub version: Option<String>,
    /// The folder, as the helper names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_folder_reads_as_no_version() {
        let empty: ExtensionFolder = serde_json::from_str(r#"{"version":null}"#).unwrap();
        assert_eq!(empty, ExtensionFolder::default());
        let full: ExtensionFolder =
            serde_json::from_str(r#"{"version":"1.2.3","path":"/x/Extension"}"#).unwrap();
        assert_eq!(full.version.as_deref(), Some("1.2.3"));
        assert_eq!(
            serde_json::to_string(&ExtensionFolder::default()).unwrap(),
            r#"{"version":null}"#
        );
    }
}
