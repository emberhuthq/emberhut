//! `cargo xtask bundle-macos`: build, bundle, sign, notarize, zip and install, in that order.
//! The bundle holds the hut's server, `Contents/MacOS/emberhut-server`, unless `--no-hut`
//! (ADR 0030).
//!
//! Everything specific to the macOS bundle lives in this module's children; the command runner,
//! the workspace paths and the progress output it uses are shared with the other tasks.

mod app;
mod args;
mod assemble;
pub(crate) mod build;
mod install;
pub(crate) mod ownership;
pub(crate) mod plan;
pub(crate) mod plist;
pub(crate) mod sign;
pub(crate) mod tool;
pub(crate) mod validate;
mod xml;

use std::fs;

use anyhow::{Context as _, bail};

use self::app::{Layout, VERSION, arch_label};
pub(crate) use self::args::BundleArgs;
use crate::say;
use crate::workspace::Workspace;

/// Runs `bundle-macos` with `args`.
///
/// Everything that can be checked up front (the OS, the options, the icon, the install
/// destination) is checked before the build starts.
///
/// # Errors
///
/// Fails when not on macOS, the options don't combine, or any step fails.
pub(crate) fn run(args: &BundleArgs) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        bail!(
            "bundle-macos makes a macOS app bundle and needs macOS and its tools (codesign, \
             ditto, iconutil, sips, xcrun); this is {}",
            std::env::consts::OS
        );
    }
    let plan = args.plan()?;
    let layout = Layout::new(Workspace::discover()?);
    let icon = layout.icon_source();
    if !icon.is_file() {
        bail!("the app icon {} is missing", icon.display());
    }
    if plan.install {
        install::preflight()?;
    }

    let built = build::build(&plan, &layout)?;
    let out_dir = layout.out_dir();
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    assemble::assemble(&layout, &built)?;
    let app = layout.app_bundle();
    // Nested code is signed before the bundle that seals it, with the same identity and the
    // hardened runtime, as notarization asks.
    if built.server.is_some() {
        sign::sign(&plan.signing, &layout.bundled_server(), None)?;
    }
    sign::sign(&plan.signing, &app, None)?;
    if let Some(notarization) = &plan.notarization {
        sign::notarize(notarization, &layout)?;
    }

    let zip = layout.zip(VERSION, arch_label(std::env::consts::ARCH));
    sign::zip_bundle(&app, &zip)?;
    say(&format!("Zipped {}", zip.display()));

    if plan.install {
        let installed = install::install(&app)?;
        say(&format!("Installed {}", installed.display()));
    } else {
        say(&format!(
            "Built {}; --install copies it to /Applications",
            app.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BundleArgs, run};

    #[test]
    #[cfg_attr(target_os = "macos", ignore = "would build and bundle the app")]
    fn it_refuses_to_run_off_macos() {
        let args = BundleArgs {
            install: true,
            ..BundleArgs::bare()
        };
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("needs macOS"), "{err}");
    }
}
