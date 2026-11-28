//! The disk image: the signed app and a link to /Applications in a compressed, read-only image
//! (UDZO, HFS+), itself signed, notarized and stapled when the app is, then checked with
//! Gatekeeper's own assessment when a desktop session is there to run it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::Context as _;

use crate::bundle_macos::app::{BUNDLE_DIR, DISPLAY_NAME, Layout};
use crate::bundle_macos::plan::{BundlePlan, Signing};
use crate::bundle_macos::sign;
use crate::bundle_macos::tool;
use crate::cmd;
use crate::files::{remove_dir_if_present, remove_file_if_present};
use crate::say;

/// How many times `hdiutil create` is tried: it sometimes fails at once with "Resource busy"
/// while the system indexes the new image, and works on the next try.
const HDIUTIL_TRIES: u32 = 3;

/// The `hdiutil create` command that images `staging` into `dmg`.
#[must_use]
pub(crate) fn hdiutil_create_command(staging: &Path, dmg: &Path) -> Command {
    let mut command = Command::new(tool::HDIUTIL);
    command
        .args(["create", "-volname", DISPLAY_NAME, "-srcfolder"])
        .arg(staging)
        .args(["-fs", "HFS+", "-format", "UDZO", "-ov"])
        .arg(dmg);
    command
}

/// The `codesign` command that signs the disk image with `identity` (with a secure timestamp;
/// the hardened runtime is for code, not images).
#[must_use]
pub(crate) fn dmg_codesign_command(identity: &str, dmg: &Path) -> Command {
    let mut command = Command::new(tool::CODESIGN);
    command
        .args(["--force", "--timestamp", "--sign", identity])
        .arg(dmg);
    command
}

/// The Gatekeeper check of a notarized disk image, as a browser download would get it.
#[must_use]
pub(crate) fn spctl_open_command(dmg: &Path) -> Command {
    let mut command = Command::new(tool::SPCTL);
    command
        .args([
            "-a",
            "-t",
            "open",
            "--context",
            "context:primary-signature",
            "-v",
        ])
        .arg(dmg);
    command
}

/// The Gatekeeper check of the app itself.
#[must_use]
pub(crate) fn spctl_exec_command(app: &Path) -> Command {
    let mut command = Command::new(tool::SPCTL);
    command.args(["-a", "-t", "exec", "-vv"]).arg(app);
    command
}

/// Whether `managername` names a logged-in desktop session.
#[must_use]
pub(crate) fn is_desktop_session(managername: &str) -> bool {
    managername.trim() == "Aqua"
}

/// Whether this runs in a logged-in desktop session (not over SSH or from a launchd job), where
/// Gatekeeper's checks and the app's smoke run can be trusted to mean what they say.
#[must_use]
pub(crate) fn gui_session() -> bool {
    cmd::capture(Command::new(tool::LAUNCHCTL).arg("managername"))
        .is_ok_and(|out| out.status.success() && is_desktop_session(&out.stdout))
}

/// Makes the disk image of the assembled (and signed) app, named for `version` and `arch`:
/// signs it with the plan's identity, and notarizes and staples it when the plan notarizes.
/// Returns its path.
///
/// # Errors
///
/// Fails when staging, imaging, signing, notarizing or Gatekeeper's check fails.
pub(crate) fn make(
    plan: &BundlePlan,
    layout: &Layout,
    version: &str,
    arch: &str,
) -> anyhow::Result<PathBuf> {
    let dmg = layout.dmg(version, arch);
    let staging = layout.dmg_staging();
    remove_dir_if_present(&staging)?;
    fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
    cmd::run(
        Command::new(tool::DITTO)
            .arg(layout.app_bundle())
            .arg(staging.join(BUNDLE_DIR)),
    )?;
    link_applications(&staging)?;
    remove_file_if_present(&dmg)?;
    say(&format!("Imaging {}", dmg.display()));
    let mut tries = 0;
    loop {
        tries += 1;
        match cmd::run(&mut hdiutil_create_command(&staging, &dmg)) {
            Ok(()) => break,
            Err(err) if tries < HDIUTIL_TRIES => {
                say(&format!("{err:#}; trying again"));
                thread::sleep(Duration::from_secs(5));
            }
            Err(err) => return Err(err),
        }
    }
    remove_dir_if_present(&staging)?;

    if let Signing::Identity(identity) = &plan.signing {
        say(&format!("Signing {} as {identity}", dmg.display()));
        cmd::run(&mut dmg_codesign_command(identity, &dmg))?;
        cmd::run(
            Command::new(tool::CODESIGN)
                .args(["--verify", "--strict", "--verbose=2"])
                .arg(&dmg),
        )?;
    }
    if let Some(notarization) = &plan.notarization {
        sign::notarize_file(&dmg, notarization, &layout.notary_log())?;
        sign::staple(&dmg)?;
        say("Notarized and stapled the disk image");
        if gui_session() {
            cmd::run(&mut spctl_open_command(&dmg))?;
            cmd::run(&mut spctl_exec_command(&layout.app_bundle()))?;
            say("Gatekeeper accepts the disk image and the app");
        } else {
            say("Skipped Gatekeeper's checks (spctl): no logged-in desktop session");
        }
    }
    Ok(dmg)
}

/// The `/Applications` link a person drags the app onto.
#[cfg(unix)]
fn link_applications(staging: &Path) -> anyhow::Result<()> {
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .context("linking /Applications in the disk image")
}

/// Never called: the disk image is made on macOS only.
#[cfg(not(unix))]
fn link_applications(_staging: &Path) -> anyhow::Result<()> {
    anyhow::bail!("disk images are made on macOS")
}

/// The `hdiutil attach` command that mounts `dmg` read-only at `mount`, out of Finder's sight.
#[must_use]
pub(crate) fn attach_command(dmg: &Path, mount: &Path) -> Command {
    let mut command = Command::new(tool::HDIUTIL);
    command
        .arg("attach")
        .arg(dmg)
        .args(["-readonly", "-nobrowse", "-noautoopen", "-mountpoint"])
        .arg(mount);
    command
}

/// The `hdiutil detach` command for the image mounted at `mount`.
#[must_use]
pub(crate) fn detach_command(mount: &Path) -> Command {
    let mut command = Command::new(tool::HDIUTIL);
    command.arg("detach").arg(mount).arg("-force");
    command
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        attach_command, detach_command, dmg_codesign_command, hdiutil_create_command,
        is_desktop_session, spctl_exec_command, spctl_open_command,
    };
    use crate::cmd::describe;

    #[test]
    fn the_image_is_compressed_hfs_named_after_the_app() {
        assert_eq!(
            describe(&hdiutil_create_command(
                Path::new("/t/bundle/dmg"),
                Path::new("/t/bundle/Emberhut-0.1.0-macos-universal.dmg")
            )),
            "/usr/bin/hdiutil create -volname Emberhut -srcfolder /t/bundle/dmg -fs HFS+ \
             -format UDZO -ov /t/bundle/Emberhut-0.1.0-macos-universal.dmg"
        );
    }

    #[test]
    fn the_image_is_signed_with_a_timestamp() {
        assert_eq!(
            describe(&dmg_codesign_command(
                "Developer ID Application: Example (ABCDE12345)",
                Path::new("E.dmg")
            )),
            "/usr/bin/codesign --force --timestamp --sign \
             'Developer ID Application: Example (ABCDE12345)' E.dmg"
        );
    }

    #[test]
    fn gatekeeper_checks_the_image_as_a_download_and_the_app_as_code() {
        assert_eq!(
            describe(&spctl_open_command(Path::new("E.dmg"))),
            "/usr/sbin/spctl -a -t open --context context:primary-signature -v E.dmg"
        );
        assert_eq!(
            describe(&spctl_exec_command(Path::new("Emberhut.app"))),
            "/usr/sbin/spctl -a -t exec -vv Emberhut.app"
        );
    }

    #[test]
    fn only_aqua_is_a_desktop_session() {
        assert!(is_desktop_session("Aqua\n"));
        assert!(!is_desktop_session("Background\n"));
        assert!(!is_desktop_session("StandardIO"));
        assert!(!is_desktop_session(""));
    }

    #[test]
    fn the_image_mounts_read_only_out_of_sight() {
        assert_eq!(
            describe(&attach_command(Path::new("E.dmg"), Path::new("/tmp/m"))),
            "/usr/bin/hdiutil attach E.dmg -readonly -nobrowse -noautoopen -mountpoint /tmp/m"
        );
        assert_eq!(
            describe(&detach_command(Path::new("/tmp/m"))),
            "/usr/bin/hdiutil detach /tmp/m -force"
        );
    }
}
