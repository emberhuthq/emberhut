//! Signing, notarizing and zipping the bundle.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, bail};

use crate::bundle_macos::app::Layout;
use crate::bundle_macos::plan::{Notarization, Signing};
use crate::bundle_macos::tool;
use crate::cmd;
use crate::files::remove_file_if_present;
use crate::say;

/// What notarytool prints when Apple accepts a submission.
const ACCEPTED: &str = "status: Accepted";

/// The `codesign` command that signs `bundle` as `signing` says, with the hardened runtime and,
/// when given, the `entitlements` file.
#[must_use]
pub(crate) fn codesign_command(
    signing: &Signing,
    bundle: &Path,
    entitlements: Option<&Path>,
) -> Command {
    let mut command = Command::new(tool::CODESIGN);
    command.args(["--force", "--options", "runtime"]);
    if let Some(entitlements) = entitlements {
        command.arg("--entitlements").arg(entitlements);
    }
    // A secure timestamp needs a real identity; ad hoc signatures can't have one.
    if let Signing::Identity(_) = signing {
        command.arg("--timestamp");
    }
    command
        .args(["--sign", signing.codesign_identity()])
        .arg(bundle);
    command
}

/// Signs `bundle` (with `entitlements`, when given) and verifies the signature.
///
/// # Errors
///
/// Fails when `codesign` fails to sign or verify.
pub(crate) fn sign(
    signing: &Signing,
    bundle: &Path,
    entitlements: Option<&Path>,
) -> anyhow::Result<()> {
    match signing {
        Signing::AdHoc => {
            say(
                "Signing ad hoc (no --sign or EMBERHUT_SIGN_IDENTITY): the app runs on this Mac only",
            );
        }
        Signing::Identity(identity) => say(&format!("Signing as {identity}")),
    }
    cmd::run(&mut codesign_command(signing, bundle, entitlements))?;
    cmd::run(
        Command::new(tool::CODESIGN)
            .args(["--verify", "--strict", "--verbose=2"])
            .arg(bundle),
    )
}

/// Whether notarytool's output says the submission was accepted.
#[must_use]
pub(crate) fn notary_accepted(output: &str) -> bool {
    output.lines().any(|line| line.trim() == ACCEPTED)
}

/// Sends the bundle to Apple's notary service, waits for the verdict and staples the ticket.
///
/// # Errors
///
/// Fails when zipping, submitting or stapling fails, or Apple doesn't accept the app; the
/// error then holds notarytool's output.
pub(crate) fn notarize(notarization: &Notarization, layout: &Layout) -> anyhow::Result<()> {
    let bundle = layout.app_bundle();
    let upload = layout.notary_upload();
    zip_bundle(&bundle, &upload)?;
    say("Submitting to Apple's notary service and waiting for its verdict");
    let mut submit = Command::new(tool::XCRUN);
    submit.args(["notarytool", "submit"]).arg(&upload).args([
        "--keychain-profile",
        &notarization.keychain_profile,
        "--wait",
    ]);
    let verdict = cmd::capture(&mut submit)?;
    let text = verdict.combined();
    let log = layout.notary_log();
    fs::write(&log, &text).with_context(|| format!("writing {}", log.display()))?;
    remove_file_if_present(&upload)?;
    if !verdict.status.success() || !notary_accepted(&text) {
        bail!(
            "`{}` did not accept the app ({}):\n{}",
            cmd::describe(&submit),
            verdict.status,
            text.trim_end()
        );
    }
    cmd::run(
        Command::new(tool::XCRUN)
            .args(["stapler", "staple"])
            .arg(&bundle),
    )?;
    say("Notarized and stapled");
    Ok(())
}

/// Zips `bundle` (keeping the `Emberhut.app` directory in the archive) to `zip`, replacing it.
///
/// # Errors
///
/// Fails when an old zip can't be removed or `ditto` fails.
pub(crate) fn zip_bundle(bundle: &Path, zip: &Path) -> anyhow::Result<()> {
    remove_file_if_present(zip)?;
    cmd::run(
        Command::new(tool::DITTO)
            .args(["-c", "-k", "--keepParent"])
            .arg(bundle)
            .arg(zip),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{codesign_command, notary_accepted};
    use crate::bundle_macos::plan::Signing;
    use crate::cmd::describe;

    #[test]
    fn ad_hoc_signing_has_no_timestamp() {
        let command = codesign_command(&Signing::AdHoc, Path::new("Emberhut.app"), None);
        assert_eq!(
            describe(&command),
            "/usr/bin/codesign --force --options runtime --sign - Emberhut.app"
        );
    }

    #[test]
    fn identity_signing_is_timestamped() {
        let signing = Signing::Identity("Developer ID Application: Example".into());
        let command = codesign_command(&signing, Path::new("Emberhut.app"), None);
        assert_eq!(
            describe(&command),
            "/usr/bin/codesign --force --options runtime --timestamp \
             --sign 'Developer ID Application: Example' Emberhut.app"
        );
    }

    #[test]
    fn entitlements_are_signed_in_when_given() {
        let command = codesign_command(
            &Signing::AdHoc,
            Path::new("Emberhut Hands.app"),
            Some(Path::new("hands.entitlements")),
        );
        assert_eq!(
            describe(&command),
            "/usr/bin/codesign --force --options runtime --entitlements hands.entitlements \
             --sign - 'Emberhut Hands.app'"
        );
    }

    #[test]
    fn only_an_accepted_status_counts() {
        let accepted =
            "Processing complete\n  id: 00000000-0000-0000-0000-000000000000\n  status: Accepted\n";
        assert!(notary_accepted(accepted));
        assert!(!notary_accepted("  status: Invalid\n"));
        assert!(!notary_accepted("  status: Accepted-ish\n"));
        assert!(!notary_accepted(""));
    }
}
