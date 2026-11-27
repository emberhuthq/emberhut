//! `cargo xtask bundle-hands`: build Emberhut Hands (`apps/hands`), bundle it as
//! `Emberhut Hands.app` with its icon and entitlements, sign it, zip it, and optionally install it
//! to `~/Applications` with its launch agent (ADR 0024). The signing, the build command and the
//! plist writer are `bundle-macos`'s.

mod app;
mod args;
mod files;
mod install;

use std::fs;

use anyhow::{Context as _, bail};

use self::app::{BUNDLE_DIR, EXECUTABLE, ICON_NAME, Layout, PACKAGE, VERSION};
pub(crate) use self::args::HandsArgs;
use crate::bundle_macos::build::{CargoBuild, cargo_build, executable};
use crate::bundle_macos::plan::BuildProfile;
use crate::bundle_macos::sign;
use crate::bundle_macos::validate::signing;
use crate::files::remove_dir_if_present;
use crate::workspace::{self, Workspace};
use crate::{cmd, say};

/// Runs `bundle-hands` with `args`.
///
/// # Errors
///
/// Fails when not on macOS, the options don't combine, or any step fails.
pub(crate) fn run(args: &HandsArgs) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        bail!(
            "bundle-hands makes a macOS app bundle and needs macOS and its tools (codesign, \
             ditto, launchctl); this is {}",
            std::env::consts::OS
        );
    }
    let signing = signing(args.sign.as_deref())?;
    let layout = Layout::new(Workspace::discover()?);
    let profile = if args.dist {
        BuildProfile::Dist
    } else {
        BuildProfile::Release
    };
    say(&format!(
        "Building {PACKAGE} ({profile}, {} jobs, low priority)",
        args.jobs
    ));
    let build = CargoBuild {
        package: PACKAGE,
        profile,
        jobs: args.jobs,
        feature: None,
    };
    let messages = cmd::stdout(&mut cargo_build(
        workspace::cargo(),
        &build,
        layout.root(),
        layout.target(),
    ))?;
    let binary = executable(&messages, EXECUTABLE)
        .with_context(|| format!("cargo built {PACKAGE} but named no `{EXECUTABLE}` binary"))?;

    let bundle = layout.app_bundle();
    say(&format!("Assembling {}", bundle.display()));
    remove_dir_if_present(&bundle)?;
    let macos = bundle.join("Contents").join("MacOS");
    fs::create_dir_all(&macos).with_context(|| format!("creating {}", macos.display()))?;
    let program = macos.join(EXECUTABLE);
    fs::copy(&binary, &program)
        .with_context(|| format!("copying {} to {}", binary.display(), program.display()))?;
    let resources = bundle.join("Contents").join("Resources");
    fs::create_dir_all(&resources).with_context(|| format!("creating {}", resources.display()))?;
    let icon = layout.icon_source();
    let icns = resources.join(format!("{ICON_NAME}.icns"));
    fs::copy(&icon, &icns)
        .with_context(|| format!("copying {} to {}", icon.display(), icns.display()))?;
    let info = bundle.join("Contents").join("Info.plist");
    fs::write(&info, files::info_plist(VERSION))
        .with_context(|| format!("writing {}", info.display()))?;
    let entitlements = layout.entitlements();
    fs::write(&entitlements, files::entitlements())
        .with_context(|| format!("writing {}", entitlements.display()))?;
    sign::sign(&signing, &bundle, Some(&entitlements))?;

    let zip = layout.zip();
    sign::zip_bundle(&bundle, &zip)?;
    say(&format!("Zipped {}", zip.display()));

    if args.install {
        let agent = args.agent()?;
        let installed = install::install(&bundle, &agent)?;
        say(&format!(
            "Installed {} and started its launch agent. The first time, turn on Emberhut Hands in \
             System Settings → Privacy & Security → Accessibility and Screen Recording.",
            installed.display()
        ));
    } else {
        say(&format!(
            "Built {}; --install puts it in ~/Applications/{BUNDLE_DIR} and starts it",
            bundle.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HandsArgs, run};

    #[test]
    #[cfg_attr(target_os = "macos", ignore = "would build and bundle the helper")]
    fn it_refuses_to_run_off_macos() {
        let err = run(&HandsArgs::bare()).unwrap_err().to_string();
        assert!(err.contains("needs macOS"), "{err}");
    }
}
