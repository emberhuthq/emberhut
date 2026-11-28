//! Checking how the `bundle-macos` options combine, before anything runs.

use std::fmt;

use crate::bundle_macos::app::VERSION;
use crate::bundle_macos::args::BundleArgs;
use crate::bundle_macos::plan::{BuildProfile, BundlePlan, BundleVersion, Notarization, Signing};

/// A combination of `bundle-macos` options that can't work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlagError {
    /// `--sign` (or `EMBERHUT_SIGN_IDENTITY`) was given but empty.
    EmptyIdentity,
    /// `--notarize` without a real signing identity: Apple rejects ad hoc signatures.
    NotarizeNeedsIdentity,
    /// `--notarize` without a keychain profile or an API key (or with an empty one).
    NotarizeNeedsProfile,
    /// `--notary-key` without its key id or issuer (or with an empty one).
    NotaryKeyIncomplete,
}

impl fmt::Display for FlagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyIdentity => {
                "the signing identity (--sign or EMBERHUT_SIGN_IDENTITY) is empty"
            }
            Self::NotarizeNeedsIdentity => {
                "--notarize needs a Developer ID signing identity (--sign or \
                 EMBERHUT_SIGN_IDENTITY); Apple does not notarize ad hoc signatures"
            }
            Self::NotarizeNeedsProfile => {
                "--notarize needs a notarytool keychain profile (--keychain-profile or \
                 EMBERHUT_NOTARY_PROFILE) or an App Store Connect API key (--notary-key)"
            }
            Self::NotaryKeyIncomplete => {
                "--notary-key needs --notary-key-id and --notary-issuer (or \
                 EMBERHUT_NOTARY_KEY_ID and EMBERHUT_NOTARY_ISSUER)"
            }
        })
    }
}

impl std::error::Error for FlagError {}

/// How `--sign` (or `EMBERHUT_SIGN_IDENTITY`) asks to sign: ad hoc without one or with `-`.
///
/// # Errors
///
/// [`FlagError::EmptyIdentity`] for an empty identity.
pub(crate) fn signing(sign: Option<&str>) -> Result<Signing, FlagError> {
    match sign.map(str::trim) {
        None | Some("-") => Ok(Signing::AdHoc),
        Some("") => Err(FlagError::EmptyIdentity),
        Some(identity) => Ok(Signing::Identity(identity.to_owned())),
    }
}

/// `value` trimmed, unless that leaves nothing.
fn filled(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

impl BundleArgs {
    /// Checks how the options combine and turns them into a plan.
    ///
    /// # Errors
    ///
    /// Returns a [`FlagError`] when the signing identity is empty, or when `--notarize` lacks
    /// a real identity or a keychain profile.
    pub(crate) fn plan(&self) -> Result<BundlePlan, FlagError> {
        let signing = signing(self.sign.as_deref())?;
        let notarization = if self.notarize {
            if signing == Signing::AdHoc {
                return Err(FlagError::NotarizeNeedsIdentity);
            }
            Some(self.notarization()?)
        } else {
            None
        };
        Ok(BundlePlan {
            profile: if self.dist {
                BuildProfile::Dist
            } else {
                BuildProfile::Release
            },
            automation: self.automation,
            jobs: self.jobs,
            hut: !self.no_hut,
            signing,
            notarization,
            install: self.install,
            universal: self.universal,
            dmg: self.dmg,
            version: BundleVersion::development(VERSION),
        })
    }

    /// The notary credentials: the API key when given, else the keychain profile.
    fn notarization(&self) -> Result<Notarization, FlagError> {
        if let Some(key) = &self.notary_key {
            return match (
                filled(self.notary_key_id.as_deref()),
                filled(self.notary_issuer.as_deref()),
            ) {
                (Some(key_id), Some(issuer)) if !key.as_os_str().is_empty() => {
                    Ok(Notarization::ApiKey {
                        key: key.clone(),
                        key_id: key_id.to_owned(),
                        issuer: issuer.to_owned(),
                    })
                }
                _ => Err(FlagError::NotaryKeyIncomplete),
            };
        }
        match filled(self.keychain_profile.as_deref()) {
            None => Err(FlagError::NotarizeNeedsProfile),
            Some(profile) => Ok(Notarization::Profile {
                keychain_profile: profile.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FlagError;
    use crate::bundle_macos::args::BundleArgs;
    use crate::bundle_macos::plan::{BuildProfile, Notarization, Signing};

    #[test]
    fn the_defaults_plan_an_ad_hoc_release_build() {
        let plan = BundleArgs::bare().plan().unwrap();
        assert_eq!(plan.profile, BuildProfile::Release);
        assert_eq!(plan.signing, Signing::AdHoc);
        assert_eq!(plan.signing.codesign_identity(), "-");
        assert_eq!(plan.notarization, None);
        assert!(!plan.automation && !plan.install);
        assert!(plan.hut, "the hut is in the bundle unless --no-hut");
        assert_eq!(plan.jobs, 4);
        let plan = BundleArgs {
            no_hut: true,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        assert!(!plan.hut);
    }

    #[test]
    fn dist_picks_the_dist_profile() {
        let plan = BundleArgs {
            dist: true,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        assert_eq!(plan.profile, BuildProfile::Dist);
        assert_eq!(plan.profile.name(), "dist");
        assert_eq!(BuildProfile::Release.to_string(), "release");
    }

    #[test]
    fn an_identity_is_used_as_given_and_a_dash_means_ad_hoc() {
        let sign = |id: &str| {
            BundleArgs {
                sign: Some(id.into()),
                ..BundleArgs::bare()
            }
            .plan()
        };
        let plan = sign("Developer ID Application: Example (ABCDE12345)").unwrap();
        assert_eq!(
            plan.signing.codesign_identity(),
            "Developer ID Application: Example (ABCDE12345)"
        );
        assert_eq!(sign("-").unwrap().signing, Signing::AdHoc);
        assert_eq!(sign("  ").unwrap_err(), FlagError::EmptyIdentity);
    }

    #[test]
    fn notarizing_needs_a_real_identity() {
        let notarize = BundleArgs {
            notarize: true,
            keychain_profile: Some("example-profile".into()),
            ..BundleArgs::bare()
        };
        assert_eq!(
            notarize.plan().unwrap_err(),
            FlagError::NotarizeNeedsIdentity
        );
        let ad_hoc = BundleArgs {
            sign: Some("-".into()),
            ..notarize
        };
        assert_eq!(ad_hoc.plan().unwrap_err(), FlagError::NotarizeNeedsIdentity);
    }

    #[test]
    fn notarizing_needs_a_profile() {
        let notarize = BundleArgs {
            notarize: true,
            sign: Some("Example".into()),
            ..BundleArgs::bare()
        };
        assert_eq!(
            notarize.clone().plan().unwrap_err(),
            FlagError::NotarizeNeedsProfile
        );
        let empty = BundleArgs {
            keychain_profile: Some(String::new()),
            ..notarize
        };
        assert_eq!(empty.plan().unwrap_err(), FlagError::NotarizeNeedsProfile);
    }

    #[test]
    fn a_full_notarized_plan() {
        let plan = BundleArgs {
            notarize: true,
            sign: Some("Example".into()),
            keychain_profile: Some("example-profile".into()),
            install: true,
            automation: true,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        assert_eq!(
            plan.notarization,
            Some(Notarization::Profile {
                keychain_profile: "example-profile".into()
            })
        );
        assert!(plan.install && plan.automation);
    }

    #[test]
    fn an_api_key_notarizes_in_place_of_a_profile() {
        let notarize = BundleArgs {
            notarize: true,
            sign: Some("Example".into()),
            notary_key: Some("/keys/AuthKey_ABC123.p8".into()),
            notary_key_id: Some("ABC123".into()),
            notary_issuer: Some("issuer".into()),
            ..BundleArgs::bare()
        };
        assert_eq!(
            notarize.plan().unwrap().notarization,
            Some(Notarization::ApiKey {
                key: "/keys/AuthKey_ABC123.p8".into(),
                key_id: "ABC123".into(),
                issuer: "issuer".into(),
            })
        );
        let incomplete = BundleArgs {
            notary_issuer: Some(" ".into()),
            ..notarize
        };
        assert_eq!(
            incomplete.plan().unwrap_err(),
            FlagError::NotaryKeyIncomplete
        );
    }

    #[test]
    fn universal_and_dmg_are_planned_and_named() {
        let plan = BundleArgs {
            universal: true,
            dmg: true,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        assert!(plan.universal && plan.dmg);
        assert_eq!(
            plan.targets(),
            ["aarch64-apple-darwin", "x86_64-apple-darwin"]
        );
        assert_eq!(plan.arch_label(), "universal");
        assert_eq!(plan.version.release, None, "a development build by default");
        let host = BundleArgs::bare().plan().unwrap();
        assert_eq!(host.targets(), [] as [&str; 0]);
        assert_ne!(host.arch_label(), "universal");
    }

    #[test]
    fn a_profile_without_notarize_is_ignored() {
        let plan = BundleArgs {
            keychain_profile: Some("example-profile".into()),
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        assert_eq!(plan.notarization, None);
    }

    #[test]
    fn flag_errors_name_the_options() {
        assert!(
            FlagError::NotarizeNeedsProfile
                .to_string()
                .contains("--keychain-profile")
        );
        assert!(
            FlagError::NotarizeNeedsIdentity
                .to_string()
                .contains("--sign")
        );
    }
}
