//! The command line: `cargo xtask <task> [options]`.

use clap::{Parser, Subcommand};

use crate::bundle_hands::HandsArgs;
use crate::bundle_macos::BundleArgs;

/// Repository tasks for emberhut, run with `cargo xtask <task>`.
#[derive(Debug, Parser)]
#[command(name = "cargo xtask", bin_name = "cargo xtask", version, about, long_about = None)]
pub(crate) struct Cli {
    /// The task to run.
    #[command(subcommand)]
    pub(crate) task: Task,
}

/// One repository task.
#[derive(Debug, Subcommand)]
pub(crate) enum Task {
    /// Build the desktop app and package it as a signed Emberhut.app for macOS.
    ///
    /// Builds `emberhut-desktop` at low priority, assembles `target/bundle/Emberhut.app` (Info.plist,
    /// icon, binary), signs it, optionally notarizes it, zips it to
    /// `target/bundle/Emberhut-<version>-macos-<arch>.zip` and optionally installs it to
    /// /Applications/Emberhut.app. Runs on macOS only; it needs the Xcode command line tools.
    BundleMacos(BundleArgs),

    /// Build Emberhut Hands, the Mac helper, and package it as a signed `Emberhut Hands.app`.
    ///
    /// Builds `emberhut-hands`, assembles `target/bundle/Emberhut Hands.app` (Info.plist, binary),
    /// signs it with its entitlements, zips it, and optionally installs it to
    /// ~/Applications with its launch agent, com.emberhut.hands. Runs on macOS only.
    BundleHands(HandsArgs),
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::Cli;

    #[test]
    fn the_command_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn an_unknown_task_is_an_error() {
        let err = Cli::command()
            .try_get_matches_from(["cargo xtask", "bundle-mac"])
            .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}
