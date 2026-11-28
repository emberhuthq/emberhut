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

/// The `notarytool submit … --wait` command that sends `file` (a zip or a disk image) with
/// `notarization`'s credentials. An API key is passed by its path: notarytool reads it in place.
#[must_use]
pub(crate) fn notary_submit_command(file: &Path, notarization: &Notarization) -> Command {
    let mut submit = Command::new(tool::XCRUN);
    submit.args(["notarytool", "submit"]).arg(file);
    match notarization {
        Notarization::Profile { keychain_profile } => {
            submit.args(["--keychain-profile", keychain_profile]);
        }
        Notarization::ApiKey {
            key,
            key_id,
            issuer,
        } => {
            submit
                .arg("--key")
                .arg(key)
                .args(["--key-id", key_id, "--issuer", issuer]);
        }
    }
    submit.arg("--wait");
    submit
}

/// Sends `file` to Apple's notary service and waits for the verdict, which is kept in `log`.
///
/// # Errors
///
/// Fails when submitting fails or Apple doesn't accept the file; the error then holds
/// notarytool's output.
pub(crate) fn notarize_file(
    file: &Path,
    notarization: &Notarization,
    log: &Path,
) -> anyhow::Result<()> {
    if let Notarization::ApiKey { key, .. } = notarization
        && !key.is_file()
    {
        bail!("the App Store Connect API key {} is missing", key.display());
    }
    say(&format!(
        "Submitting {} to Apple's notary service and waiting for its verdict",
        file.display()
    ));
    let mut submit = notary_submit_command(file, notarization);
    let verdict = cmd::capture(&mut submit)?;
    let text = verdict.combined();
    fs::write(log, &text).with_context(|| format!("writing {}", log.display()))?;
    if !verdict.status.success() || !notary_accepted(&text) {
        bail!(
            "`{}` did not accept {} ({}):\n{}",
            cmd::describe(&submit),
            file.display(),
            verdict.status,
            text.trim_end()
        );
    }
    Ok(())
}

/// Staples the notary ticket to `path` (an app or a disk image).
///
/// # Errors
///
/// Fails when `stapler` fails.
pub(crate) fn staple(path: &Path) -> anyhow::Result<()> {
    cmd::run(
        Command::new(tool::XCRUN)
            .args(["stapler", "staple"])
            .arg(path),
    )
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
    let verdict = notarize_file(&upload, notarization, &layout.notary_log());
    remove_file_if_present(&upload)?;
    verdict?;
    staple(&bundle)?;
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

    use super::{codesign_command, notary_accepted, notary_submit_command};
    use crate::bundle_macos::plan::{Notarization, Signing};
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
    fn notarytool_gets_a_profile_or_the_api_key_in_place() {
        let profile = Notarization::Profile {
            keychain_profile: "emberhut-notary".into(),
        };
        assert_eq!(
            describe(&notary_submit_command(Path::new("a.dmg"), &profile)),
            "/usr/bin/xcrun notarytool submit a.dmg --keychain-profile emberhut-notary --wait"
        );
        let key = Notarization::ApiKey {
            key: "/keys/AuthKey_ABC123.p8".into(),
            key_id: "ABC123".into(),
            issuer: "00000000-0000-0000-0000-000000000000".into(),
        };
        assert_eq!(
            describe(&notary_submit_command(Path::new("server.zip"), &key)),
            "/usr/bin/xcrun notarytool submit server.zip --key /keys/AuthKey_ABC123.p8 \
             --key-id ABC123 --issuer 00000000-0000-0000-0000-000000000000 --wait"
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
