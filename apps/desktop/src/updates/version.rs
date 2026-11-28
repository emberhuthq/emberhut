//! This copy's version and build. A release is built in CI with `EMBERHUT_VERSION` (the tag
//! without its `v`: `0.1.0-rc.1` or `0.1.0`) and `EMBERHUT_BUILD` (the number the tag gives,
//! [`super::tag`]); every other build is a development build, which never checks for updates
//! and never installs one, and shows the workspace's version.

use std::fmt;

use emberhut_api::releases::Build;

/// The version: the release's tag without its `v`, or the workspace's `MAJOR.MINOR.PATCH`.
pub(crate) const VERSION: &str = match option_env!("EMBERHUT_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The build number the release compiled in, if it did.
const BUILD: Option<&str> = option_env!("EMBERHUT_BUILD");

/// This copy: its version, and its build when it is a published release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Running {
    /// The version.
    pub(crate) version: &'static str,
    /// The build; `None` for a development build.
    pub(crate) build: Option<Build>,
}

impl Running {
    /// The copy that is running.
    #[must_use]
    pub(crate) fn this() -> Self {
        Self::from_parts(VERSION, BUILD)
    }

    /// A copy of `version` built with `build` as `EMBERHUT_BUILD`: a development build unless it
    /// is a whole number above 0.
    #[must_use]
    pub(crate) fn from_parts(version: &'static str, build: Option<&str>) -> Self {
        let build = build
            .and_then(|text| text.trim().parse::<u64>().ok())
            .filter(|&build| build > 0)
            .map(Build::new);
        Self { version, build }
    }

    /// Whether this copy is a release candidate (`0.1.0-rc.1`).
    #[must_use]
    pub(crate) fn is_candidate(&self) -> bool {
        is_candidate(self.version)
    }
}

/// Whether `version` names a release candidate (`0.1.0-rc.1`).
#[must_use]
pub(crate) fn is_candidate(version: &str) -> bool {
    version.contains("-rc.")
}

/// `0.2.0 (build 12)`, or `0.2.0 (development build)`.
impl fmt::Display for Running {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.build {
            Some(build) => write!(f, "{} (build {build})", self.version),
            None => write!(f, "{} (development build)", self.version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_reads_with_its_build_and_anything_else_is_a_development_build() {
        let release = Running::from_parts("0.2.0", Some("12"));
        assert_eq!(release.build, Some(Build::new(12)));
        assert_eq!(release.to_string(), "0.2.0 (build 12)");
        for build in [None, Some(""), Some("0"), Some("twelve"), Some("-3")] {
            let copy = Running::from_parts("0.1.0", build);
            assert_eq!(copy.build, None, "{build:?}");
            assert_eq!(copy.to_string(), "0.1.0 (development build)");
        }
    }

    #[test]
    fn this_copy_is_the_releases_version_or_the_workspaces() {
        let expected = option_env!("EMBERHUT_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
        assert_eq!(Running::this().version, expected);
    }

    #[test]
    fn a_candidate_is_told_by_its_version() {
        assert!(Running::from_parts("0.1.0-rc.1", Some("10001")).is_candidate());
        assert!(!Running::from_parts("0.1.0", Some("10099")).is_candidate());
        assert!(!is_candidate("0.1.0"));
    }
}
