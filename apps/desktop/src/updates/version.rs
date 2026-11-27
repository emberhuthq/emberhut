//! This copy's version and build. Only a published release is built with `EMBERHUT_BUILD` set (by
//! the release command); every other build is a development build, which never checks for
//! updates and never installs one.

use std::fmt;

use emberhut_api::releases::Build;

/// The workspace's version, plain `MAJOR.MINOR.PATCH`.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The build number the release command compiled in, if it did.
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
    fn this_copy_is_the_workspaces_version() {
        assert_eq!(Running::this().version, env!("CARGO_PKG_VERSION"));
    }
}
