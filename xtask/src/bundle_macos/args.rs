//! The options of `cargo xtask bundle-macos`, as clap parses them.

use std::path::PathBuf;

use clap::{ArgGroup, Args};

/// Options of `cargo xtask bundle-macos`, as typed; [`BundleArgs::plan`] checks how they combine.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(group(ArgGroup::new("notary").args(["keychain_profile", "notary_key"])))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent command line switch"
)]
pub(crate) struct BundleArgs {
    /// Build with the `release` profile (the default).
    #[arg(long, conflicts_with = "dist")]
    pub(crate) release: bool,

    /// Build with the `dist` profile instead: smaller and slower to build, for builds you
    /// hand out.
    #[arg(long)]
    pub(crate) dist: bool,

    /// Build with the app's `automation` feature: the build an automated test harness installs
    /// so it can measure the app and drive its window. Not for everyday use.
    #[arg(long)]
    pub(crate) automation: bool,

    /// How many compile jobs cargo may run at once. The build also runs under `nice`, so the
    /// Mac stays usable while it compiles.
    #[arg(
        long,
        short = 'j',
        value_name = "N",
        default_value_t = 4,
        value_parser = clap::value_parser!(u16).range(1..)
    )]
    pub(crate) jobs: u16,

    /// The code signing identity to sign with, as `security find-identity -v -p codesigning`
    /// lists it (a "Developer ID Application: ..." name or its SHA-1 hash). Without one the
    /// app is signed ad hoc, which runs on this Mac only.
    #[arg(long, value_name = "IDENTITY", env = "EMBERHUT_SIGN_IDENTITY")]
    pub(crate) sign: Option<String>,

    /// Send the signed app (and the disk image, with --dmg) to Apple's notary service and
    /// staple the ticket to it, so it opens on other Macs without a warning. Needs --sign with a
    /// Developer ID identity, and --keychain-profile or --notary-key.
    #[arg(long, requires = "notary")]
    pub(crate) notarize: bool,

    /// The notarytool keychain profile with your App Store Connect credentials, made once with
    /// `xcrun notarytool store-credentials <name>`. Used only with --notarize.
    #[arg(long, value_name = "NAME", env = "EMBERHUT_NOTARY_PROFILE")]
    pub(crate) keychain_profile: Option<String>,

    /// Notarize with this App Store Connect API key (`AuthKey_<id>.p8`) instead of a keychain
    /// profile. notarytool reads it where it is; it is never copied. Needs --notary-key-id and
    /// --notary-issuer.
    #[arg(
        long,
        value_name = "PATH",
        env = "EMBERHUT_NOTARY_KEY",
        conflicts_with = "keychain_profile",
        requires_all = ["notary_key_id", "notary_issuer"]
    )]
    pub(crate) notary_key: Option<PathBuf>,

    /// The App Store Connect API key's id, with --notary-key.
    #[arg(long, value_name = "ID", env = "EMBERHUT_NOTARY_KEY_ID")]
    pub(crate) notary_key_id: Option<String>,

    /// The App Store Connect issuer id of the API key, with --notary-key.
    #[arg(long, value_name = "UUID", env = "EMBERHUT_NOTARY_ISSUER")]
    pub(crate) notary_issuer: Option<String>,

    /// Build for Apple silicon and Intel (aarch64- and x86_64-apple-darwin) and join each binary
    /// with `lipo`, so one app runs on both. Needs both Rust targets installed
    /// (`rustup target add aarch64-apple-darwin x86_64-apple-darwin`).
    #[arg(long)]
    pub(crate) universal: bool,

    /// Also make a disk image, `target/bundle/Emberhut-<version>-macos-<arch>.dmg` (`universal`
    /// with --universal), holding the app and a link to /Applications. It is signed with
    /// --sign's identity and notarized with --notarize.
    #[arg(long)]
    pub(crate) dmg: bool,

    /// Leave the hut out: build and bundle the app alone, without the `emberhut-server` it
    /// runs on this Mac (ADR 0030). Quicker for a build that talks to a server elsewhere; such
    /// an app can't set up its own hut.
    #[arg(long)]
    pub(crate) no_hut: bool,

    /// Copy the app to /Applications/Emberhut.app. An existing /Applications/Emberhut.app is replaced
    /// only when it is this app (bundle id com.emberhut.desktop); anything else there is left alone
    /// and the task fails.
    #[arg(long)]
    pub(crate) install: bool,
}

#[cfg(test)]
impl BundleArgs {
    /// The options of a bare `cargo xtask bundle-macos` with no `EMBERHUT_*` variables set.
    pub(crate) fn bare() -> Self {
        Self {
            release: false,
            dist: false,
            automation: false,
            jobs: 4,
            sign: None,
            notarize: false,
            keychain_profile: None,
            notary_key: None,
            notary_key_id: None,
            notary_issuer: None,
            universal: false,
            dmg: false,
            no_hut: false,
            install: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, FromArgMatches};

    use super::BundleArgs;
    use crate::cli::{Cli, Task};

    /// Parses `args` with the env fallbacks switched off, so the result doesn't depend on the
    /// environment the tests run in.
    fn parse(args: &[&str]) -> Result<BundleArgs, clap::Error> {
        let command = Cli::command().mut_subcommand("bundle-macos", |sub| {
            [
                "sign",
                "keychain_profile",
                "notary_key",
                "notary_key_id",
                "notary_issuer",
            ]
            .into_iter()
            .fold(sub, |sub, id| sub.mut_arg(id, |arg| arg.env(None::<&str>)))
        });
        let matches =
            command.try_get_matches_from(["cargo xtask", "bundle-macos"].iter().chain(args))?;
        match Cli::from_arg_matches(&matches)?.task {
            Task::BundleMacos(bundle) => Ok(bundle),
            Task::BundleHands(_) | Task::WindowsInstaller(_) | Task::Release(_) => {
                panic!("bundle-macos")
            }
        }
    }

    #[test]
    fn defaults_are_a_release_build_with_four_jobs_and_nothing_else() {
        assert_eq!(parse(&[]).unwrap(), BundleArgs::bare());
    }

    #[test]
    fn every_option_parses() {
        let args = parse(&[
            "--dist",
            "--automation",
            "--jobs",
            "2",
            "--sign",
            "Developer ID Application: Example (ABCDE12345)",
            "--notarize",
            "--keychain-profile",
            "example-profile",
            "--no-hut",
            "--install",
        ])
        .unwrap();
        assert!(args.dist && args.automation && args.notarize && args.install && args.no_hut);
        assert_eq!(args.jobs, 2);
        assert_eq!(
            args.sign.as_deref(),
            Some("Developer ID Application: Example (ABCDE12345)")
        );
        assert_eq!(args.keychain_profile.as_deref(), Some("example-profile"));
    }

    #[test]
    fn release_and_dist_conflict() {
        let err = parse(&["--release", "--dist"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn notarize_needs_a_keychain_profile() {
        let err = parse(&["--notarize", "--sign", "Example"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn a_universal_signed_and_notarized_disk_image_with_an_api_key() {
        let args = parse(&[
            "--dist",
            "--universal",
            "--dmg",
            "--sign",
            "Example",
            "--notarize",
            "--notary-key",
            "/keys/AuthKey_ABC123.p8",
            "--notary-key-id",
            "ABC123",
            "--notary-issuer",
            "00000000-0000-0000-0000-000000000000",
        ])
        .unwrap();
        assert!(args.universal && args.dmg && args.notarize);
        assert_eq!(
            args.notary_key.as_deref(),
            Some(std::path::Path::new("/keys/AuthKey_ABC123.p8"))
        );
        assert_eq!(args.notary_key_id.as_deref(), Some("ABC123"));
    }

    #[test]
    fn an_api_key_needs_its_id_and_issuer_and_excludes_a_profile() {
        let err = parse(&["--notarize", "--notary-key", "k.p8"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        let err = parse(&[
            "--notarize",
            "--keychain-profile",
            "p",
            "--notary-key",
            "k.p8",
            "--notary-key-id",
            "i",
            "--notary-issuer",
            "u",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn jobs_must_be_at_least_one() {
        let err = parse(&["--jobs", "0"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }
}
