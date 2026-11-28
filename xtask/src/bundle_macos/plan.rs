//! What `bundle-macos` will do, decided from its options before anything runs.

use std::fmt;
use std::path::PathBuf;

use crate::release::tag::Tag;

/// The cargo profile the app is built with; its name is also its directory under `target/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildProfile {
    /// `--profile release`, the default.
    Release,
    /// `--profile dist`, the workspace's size-first profile for shipped builds.
    Dist,
}

impl BuildProfile {
    /// The profile's name, as `cargo --profile` takes it and as its output directory is named.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Dist => "dist",
        }
    }
}

impl fmt::Display for BuildProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How the bundle is code signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Signing {
    /// An ad hoc signature (`codesign --sign -`): the app runs on the Mac that built it.
    AdHoc,
    /// A signing identity from the keychain, such as a Developer ID Application certificate.
    Identity(String),
}

impl Signing {
    /// The value `codesign --sign` takes for this signing.
    #[must_use]
    pub(crate) fn codesign_identity(&self) -> &str {
        match self {
            Self::AdHoc => "-",
            Self::Identity(identity) => identity,
        }
    }
}

/// Notarization through `xcrun notarytool`, and the App Store Connect credentials it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Notarization {
    /// A notarytool keychain profile holding the credentials.
    Profile {
        /// The profile's name.
        keychain_profile: String,
    },
    /// An App Store Connect API key, read in place by notarytool (never copied).
    ApiKey {
        /// The `AuthKey_<id>.p8` file.
        key: PathBuf,
        /// The key's id.
        key_id: String,
        /// The issuer id of the team's keys.
        issuer: String,
    },
}

/// The macOS targets a universal build is made of, in the order `lipo` joins them.
pub(crate) const UNIVERSAL_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

/// The versions the bundle carries, and whether this is a release build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundleVersion {
    /// The full version (`EmberhutVersion`, and the file names): `0.1.0-rc.1`.
    pub(crate) full: String,
    /// `CFBundleShortVersionString`: plain `X.Y.Z`, the only form Apple accepts there.
    pub(crate) short: String,
    /// `CFBundleVersion`: the build number for a release, else the version.
    pub(crate) bundle: String,
    /// A release build's version and build number, passed to the compiler as
    /// `EMBERHUT_VERSION` and `EMBERHUT_BUILD`; `None` for a development build.
    pub(crate) release: Option<(String, u64)>,
}

/// Why `EMBERHUT_VERSION` and `EMBERHUT_BUILD` don't describe a release of this workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionError(pub(crate) String);

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VersionError {}

impl BundleVersion {
    /// A development build of the workspace's `version`.
    #[must_use]
    pub(crate) fn development(version: &str) -> Self {
        Self {
            full: version.to_owned(),
            short: version.to_owned(),
            bundle: version.to_owned(),
            release: None,
        }
    }

    /// The release build of `tag`.
    #[must_use]
    pub(crate) fn release(tag: &Tag) -> Self {
        Self {
            full: tag.version(),
            short: tag.base(),
            bundle: tag.build().to_string(),
            release: Some((tag.version(), tag.build())),
        }
    }

    /// The version `EMBERHUT_VERSION` and `EMBERHUT_BUILD` ask for, given as `version` and
    /// `build`, of a workspace at `workspace`: a development build when neither is set.
    ///
    /// # Errors
    ///
    /// Fails when only one is set, the version isn't a release tag's, its base isn't the
    /// workspace's, or the build isn't the one the tag derives.
    pub(crate) fn resolve(
        version: Option<&str>,
        build: Option<&str>,
        workspace: &str,
    ) -> Result<Self, VersionError> {
        let (version, build) = match (version, build) {
            (None, None) => return Ok(Self::development(workspace)),
            (Some(version), Some(build)) => (version, build),
            _ => {
                return Err(VersionError(
                    "set EMBERHUT_VERSION and EMBERHUT_BUILD together (`cargo xtask release \
                     version <tag>` prints both), or neither"
                        .to_owned(),
                ));
            }
        };
        let tag = Tag::parse(&format!("v{version}"))
            .map_err(|err| VersionError(format!("EMBERHUT_VERSION: {err}")))?;
        if tag.base() != workspace {
            return Err(VersionError(format!(
                "EMBERHUT_VERSION {version} isn't a release of the workspace's version {workspace}"
            )));
        }
        if build.trim() != tag.build().to_string() {
            return Err(VersionError(format!(
                "EMBERHUT_BUILD is {build}, but {version}'s build is {}",
                tag.build()
            )));
        }
        Ok(Self::release(&tag))
    }
}

/// A checked set of `bundle-macos` options.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent command line switch"
)]
pub(crate) struct BundlePlan {
    /// The cargo profile to build.
    pub(crate) profile: BuildProfile,
    /// Whether to build with the app's `automation` feature.
    pub(crate) automation: bool,
    /// How many compile jobs cargo may run at once.
    pub(crate) jobs: u16,
    /// Whether the hut's server is built and bundled with the app.
    pub(crate) hut: bool,
    /// How to sign the bundle.
    pub(crate) signing: Signing,
    /// Whether, and how, to notarize the signed bundle (and the disk image).
    pub(crate) notarization: Option<Notarization>,
    /// Whether to install the bundle to /Applications.
    pub(crate) install: bool,
    /// Whether to build for both Mac processors and join the binaries with `lipo`.
    pub(crate) universal: bool,
    /// Whether to make a disk image with the app and an /Applications link.
    pub(crate) dmg: bool,
    /// The versions the bundle carries.
    pub(crate) version: BundleVersion,
}

impl BundlePlan {
    /// The `--target`s cargo builds: none (the host's) unless universal.
    #[must_use]
    pub(crate) fn targets(&self) -> &'static [&'static str] {
        if self.universal {
            &UNIVERSAL_TARGETS
        } else {
            &[]
        }
    }

    /// The architecture the file names carry: `universal`, or this Mac's as Apple names it.
    #[must_use]
    pub(crate) fn arch_label(&self) -> &'static str {
        if self.universal {
            "universal"
        } else {
            crate::bundle_macos::app::arch_label(std::env::consts::ARCH)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildProfile, BundleVersion, Signing};
    use crate::release::tag::Tag;

    #[test]
    fn profiles_are_named_as_cargo_names_them() {
        assert_eq!(BuildProfile::Release.name(), "release");
        assert_eq!(BuildProfile::Dist.to_string(), "dist");
    }

    #[test]
    fn ad_hoc_signing_is_a_dash() {
        assert_eq!(Signing::AdHoc.codesign_identity(), "-");
        assert_eq!(
            Signing::Identity("Example".into()).codesign_identity(),
            "Example"
        );
    }

    #[test]
    fn without_the_variables_it_is_a_development_build() {
        let version = BundleVersion::resolve(None, None, "0.1.0").unwrap();
        assert_eq!(version, BundleVersion::development("0.1.0"));
        assert_eq!(version.bundle, "0.1.0");
        assert_eq!(version.release, None);
    }

    #[test]
    fn a_release_carries_the_plain_version_the_build_and_the_full_version() {
        let version = BundleVersion::resolve(Some("0.1.0-rc.2"), Some("10002"), "0.1.0").unwrap();
        assert_eq!(version.short, "0.1.0");
        assert_eq!(version.bundle, "10002");
        assert_eq!(version.full, "0.1.0-rc.2");
        assert_eq!(version.release, Some(("0.1.0-rc.2".to_owned(), 10002)));
        assert_eq!(
            version,
            BundleVersion::release(&Tag::parse("v0.1.0-rc.2").unwrap())
        );
    }

    #[test]
    fn the_variables_must_agree_with_each_other_and_the_workspace() {
        let err = |version, build| {
            BundleVersion::resolve(version, build, "0.1.0")
                .unwrap_err()
                .to_string()
        };
        assert!(err(Some("0.1.0"), None).contains("together"));
        assert!(err(None, Some("10099")).contains("together"));
        assert!(err(Some("0.2.0"), Some("20099")).contains("workspace's version 0.1.0"));
        assert!(err(Some("0.1.0"), Some("10001")).contains("build is 10099"));
        assert!(err(Some("0.1"), Some("1")).contains("not a release tag"));
    }
}
