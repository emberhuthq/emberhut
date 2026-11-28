//! The new bundle's `Info.plist`: it must be this app (`com.emberhut.desktop`), of the release's
//! version (`EmberhutVersion` the whole of it, `0.1.0-rc.1`, and `CFBundleShortVersionString` its
//! plain numbers, `0.1.0`, the only form Apple takes there), with `CFBundleVersion` the release's
//! build.

use std::path::Path;

use emberhut_api::releases::Build;

use super::tools::{self, ToolError};
use crate::brand::APP_ID;

/// The `plutil` tool.
const PLUTIL: &str = "/usr/bin/plutil";

/// What the bundle says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BundleInfo {
    /// `CFBundleIdentifier`.
    pub(crate) id: String,
    /// `CFBundleShortVersionString`.
    pub(crate) version: String,
    /// `EmberhutVersion`, the whole version, if the bundle has one.
    pub(crate) full: Option<String>,
    /// `CFBundleVersion`.
    pub(crate) build: String,
}

/// Why a bundle isn't the release's: it names another app, version or build.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlistError {
    /// It names another app, version or build.
    #[error("the new bundle is {found}, not {wanted}")]
    Mismatch {
        /// What it says.
        found: String,
        /// What the release says.
        wanted: String,
    },
}

/// Reads the bundle at `app`'s `Info.plist`.
///
/// # Errors
///
/// It couldn't be read.
pub(crate) fn read(app: &Path) -> Result<BundleInfo, ToolError> {
    let plist = app.join("Contents/Info.plist");
    let key = |key: &str| {
        tools::run(
            PLUTIL,
            [
                "-extract".as_ref(),
                key.as_ref(),
                "raw".as_ref(),
                "-o".as_ref(),
                "-".as_ref(),
                plist.as_os_str(),
            ],
        )
        .map(|said| said.out.trim().to_owned())
    };
    Ok(BundleInfo {
        id: key("CFBundleIdentifier")?,
        version: key("CFBundleShortVersionString")?,
        full: key("EmberhutVersion").ok(),
        build: key("CFBundleVersion")?,
    })
}

/// Checks that `info` is this app, at a release's `version` and `build`.
///
/// # Errors
///
/// [`PlistError::Mismatch`] with what it is.
pub(crate) fn matches(info: &BundleInfo, version: &str, build: Build) -> Result<(), PlistError> {
    let plain = version.split_once('-').map_or(version, |(plain, _)| plain);
    let wanted = BundleInfo {
        id: APP_ID.to_owned(),
        version: plain.to_owned(),
        full: Some(version.to_owned()),
        build: build.to_string(),
    };
    if *info == wanted {
        Ok(())
    } else {
        let show = |info: &BundleInfo| {
            format!(
                "{} {} build {}",
                info.id,
                info.full.as_deref().unwrap_or(&info.version),
                info.build
            )
        };
        Err(PlistError::Mismatch {
            found: show(info),
            wanted: show(&wanted),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_must_be_this_app_at_the_releases_version_and_build() {
        let release = ("0.3.0", Build::new(15));
        let matches =
            |info: &BundleInfo, &(version, build): &(&str, Build)| matches(info, version, build);
        let mut info = BundleInfo {
            id: "com.emberhut.desktop".into(),
            version: "0.3.0".into(),
            full: Some("0.3.0".into()),
            build: "15".into(),
        };
        matches(&info, &release).unwrap();
        info.build = "14".into();
        let error = matches(&info, &release).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the new bundle is com.emberhut.desktop 0.3.0 build 14, not com.emberhut.desktop 0.3.0 build 15"
        );
        info.build = "15".into();
        info.id = "com.example.other".into();
        assert!(matches(&info, &release).is_err());
    }

    #[test]
    fn a_candidates_bundle_says_its_whole_version_beside_the_plain_one() {
        let mut info = BundleInfo {
            id: "com.emberhut.desktop".into(),
            version: "0.1.0".into(),
            full: Some("0.1.0-rc.2".into()),
            build: "10002".into(),
        };
        matches(&info, "0.1.0-rc.2", Build::new(10002)).unwrap();
        assert!(
            matches(&info, "0.1.0", Build::new(10002)).is_err(),
            "the final isn't its candidate"
        );
        info.full = None;
        assert!(
            matches(&info, "0.1.0-rc.2", Build::new(10002)).is_err(),
            "a bundle without its whole version isn't taken"
        );
    }
}
