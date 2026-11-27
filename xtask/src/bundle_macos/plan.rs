//! What `bundle-macos` will do, decided from its options before anything runs.

use std::fmt;

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

/// Notarization through `xcrun notarytool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notarization {
    /// The notarytool keychain profile holding the App Store Connect credentials.
    pub(crate) keychain_profile: String,
}

/// A checked set of `bundle-macos` options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundlePlan {
    /// The cargo profile to build.
    pub(crate) profile: BuildProfile,
    /// Whether to build with the app's `automation` feature.
    pub(crate) automation: bool,
    /// How many compile jobs cargo may run at once.
    pub(crate) jobs: u16,
    /// How to sign the bundle.
    pub(crate) signing: Signing,
    /// Whether, and how, to notarize the signed bundle.
    pub(crate) notarization: Option<Notarization>,
    /// Whether to install the bundle to /Applications.
    pub(crate) install: bool,
}

#[cfg(test)]
mod tests {
    use super::{BuildProfile, Signing};

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
}
