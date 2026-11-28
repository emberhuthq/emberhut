//! `cargo xtask bundle-macos`: build, bundle, sign, notarize, zip, image and install, in that
//! order.
//! The bundle holds the hut's server, `Contents/MacOS/emberhut-server`, unless `--no-hut`
//! (ADR 0030).
//!
//! Everything specific to the macOS bundle lives in this module's children; the command runner,
//! the workspace paths and the progress output it uses are shared with the other tasks.

mod app;
mod args;
mod assemble;
pub(crate) mod build;
pub(crate) mod dmg;
mod install;
pub(crate) mod ownership;
pub(crate) mod plan;
pub(crate) mod plist;
pub(crate) mod sign;
pub(crate) mod tool;
pub(crate) mod validate;
mod xml;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, bail};

pub(crate) use self::app::Layout;
use self::app::VERSION;
pub(crate) use self::args::BundleArgs;
use self::plan::{BundlePlan, BundleVersion};
use crate::say;
use crate::workspace::Workspace;

/// What a run of `bundle-macos` made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bundled {
    /// The signed (and maybe notarized) `Emberhut.app`.
    pub(crate) app: PathBuf,
    /// Its zip.
    pub(crate) zip: PathBuf,
    /// The disk image, with `--dmg`.
    pub(crate) dmg: Option<PathBuf>,
    /// The hut's server as built (universal when the app is), before the bundle's copy was
    /// signed; `None` with `--no-hut`.
    pub(crate) server: Option<PathBuf>,
}

/// Fails off macOS, before anything runs.
///
/// # Errors
///
/// Fails when not on macOS.
pub(crate) fn require_macos(task: &str) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        bail!(
            "{task} makes a macOS app bundle and needs macOS and its tools (codesign, \
             ditto, hdiutil, iconutil, lipo, sips, xcrun); this is {}",
            std::env::consts::OS
        );
    }
    Ok(())
}

/// Runs `bundle-macos` with `args`; a release build when `EMBERHUT_VERSION` and
/// `EMBERHUT_BUILD` are set.
///
/// Everything that can be checked up front (the OS, the options, the version, the icon, the
/// install destination) is checked before the build starts.
///
/// # Errors
///
/// Fails when not on macOS, the options don't combine, or any step fails.
pub(crate) fn run(args: &BundleArgs) -> anyhow::Result<()> {
    require_macos("bundle-macos")?;
    let mut plan = args.plan()?;
    let env = |key| std::env::var(key).ok().filter(|value| !value.is_empty());
    plan.version = BundleVersion::resolve(
        env("EMBERHUT_VERSION").as_deref(),
        env("EMBERHUT_BUILD").as_deref(),
        VERSION,
    )?;
    let layout = Layout::new(Workspace::discover()?);
    bundle(&plan, &layout).map(drop)
}

/// Builds, assembles, signs, notarizes, zips, images and installs as `plan` says.
///
/// # Errors
///
/// Fails when any step fails.
pub(crate) fn bundle(plan: &BundlePlan, layout: &Layout) -> anyhow::Result<Bundled> {
    let icon = layout.icon_source();
    if !icon.is_file() {
        bail!("the app icon {} is missing", icon.display());
    }
    if plan.install {
        install::preflight()?;
    }
    if let Some((version, build)) = &plan.version.release {
        say(&format!("A release build: {version}, build {build}"));
    }

    let built = build::build(plan, layout)?;
    let out_dir = layout.out_dir();
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    assemble::assemble(layout, &built, &plan.version)?;
    let app = layout.app_bundle();
    // Nested code is signed before the bundle that seals it, with the same identity and the
    // hardened runtime, as notarization asks.
    if built.server.is_some() {
        sign::sign(&plan.signing, &layout.bundled_server(), None)?;
    }
    sign::sign(&plan.signing, &app, None)?;
    if let Some(notarization) = &plan.notarization {
        sign::notarize(notarization, layout)?;
    }

    let version = &plan.version.full;
    let zip = layout.zip(version, plan.arch_label());
    sign::zip_bundle(&app, &zip)?;
    say(&format!("Zipped {}", zip.display()));

    let dmg = if plan.dmg {
        let dmg = dmg::make(plan, layout, version, plan.arch_label())?;
        say(&format!("Made {}", dmg.display()));
        Some(dmg)
    } else {
        None
    };

    if plan.install {
        let installed = install::install(&app)?;
        say(&format!("Installed {}", installed.display()));
    } else {
        say(&format!(
            "Built {}; --install copies it to /Applications",
            app.display()
        ));
    }
    Ok(Bundled {
        app,
        zip,
        dmg,
        server: built.server,
    })
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
