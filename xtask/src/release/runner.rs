//! The runner's macOS archives, signed on the Mac. CI uploads
//! `emberhut-runner-<version>-macos-<arch>.tar.gz` to the draft unsigned (one binary, the two
//! licence files and a README in one top folder); an unsigned binary hangs behind the macOS
//! firewall, so `release mac` downloads each, signs the binary with the Developer ID identity
//! and the hardened runtime, notarizes it (as a zip: a bare binary can't be stapled, and
//! Gatekeeper's online check covers it), repacks it with exactly the same entries and uploads
//! it again, before the publish stage sums and scans it.

use std::fs::{self, DirBuilder};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail, ensure};

use crate::bundle_macos::plan::{BundlePlan, Signing};
use crate::bundle_macos::{Layout, sign, tool};
use crate::cmd;
use crate::files::{remove_dir_if_present, remove_file_if_present};
use crate::release::feed::REPOSITORY;
use crate::release::tag::Tag;
use crate::say;

/// The runner binary's code signing identifier.
pub(crate) const RUNNER_IDENTIFIER: &str = "com.emberhut.runner";

/// The runner's macOS archives among the draft's asset names in `release_json`
/// (`gh release view --json assets`), for `version`, sorted.
#[must_use]
pub(crate) fn runner_assets(release_json: &str, version: &str) -> Vec<String> {
    let prefix = format!("emberhut-runner-{version}-macos-");
    let Ok(release) = serde_json::from_str::<serde_json::Value>(release_json) else {
        return Vec::new();
    };
    let mut names: Vec<String> = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| asset["name"].as_str())
        .filter(|name| {
            name.strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".tar.gz"))
                .is_some_and(|arch| {
                    !arch.is_empty() && arch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
        })
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

/// The entries `tar -tzf` lists, one a line.
#[must_use]
pub(crate) fn entries(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The runner binary's file name in its archive.
pub(crate) const RUNNER_BINARY: &str = "emberhut-runner";

/// The entry of `entries` that is the binary: `<top folder>/emberhut-runner`, the only thing
/// that is signed (the licences and the README are left as they are).
///
/// # Errors
///
/// Fails unless the archive holds one top folder with exactly one such binary in it.
pub(crate) fn binary_entry(entries: &[String]) -> anyhow::Result<String> {
    let tops: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.split('/').next())
        .collect();
    ensure!(
        !tops.is_empty() && tops.iter().all(|top| *top == tops[0]),
        "the archive doesn't hold one top folder"
    );
    let binary = format!("{}/{RUNNER_BINARY}", tops[0]);
    let found = entries.iter().filter(|entry| **entry == binary).count();
    ensure!(found == 1, "the archive holds {found} `{binary}`, not one");
    Ok(binary)
}

/// The `codesign` command that signs the runner binary at `path` as `identity`.
#[must_use]
pub(crate) fn codesign_command(identity: &str, path: &Path) -> Command {
    let mut command = Command::new(tool::CODESIGN);
    command
        .args(["--force", "--options", "runtime", "--timestamp"])
        .args(["--identifier", RUNNER_IDENTIFIER, "--sign", identity])
        .arg(path);
    command
}

/// The `tar` command that packs exactly `entries`, in their order and without recursing into
/// folders, from `root` into `archive`.
#[must_use]
pub(crate) fn repack_command(archive: &Path, root: &Path, entries: &[String]) -> Command {
    let mut command = Command::new(tool::TAR);
    command
        .env("COPYFILE_DISABLE", "1")
        // `--no-recursion` (bsdtar and GNU tar alike): only the entries named, so a folder adds
        // nothing that wasn't in the original.
        .args(["-cz", "--no-recursion", "-f"])
        .arg(archive)
        .arg("-C")
        .arg(root)
        .args(entries);
    command
}

/// A folder only its owner may enter, made afresh.
fn private_dir(path: &Path) -> anyhow::Result<()> {
    remove_dir_if_present(path)?;
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("creating {}", path.display()))
}

/// The permission bits of the file at `path`.
#[cfg(unix)]
fn file_mode(path: &Path) -> anyhow::Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .permissions()
        .mode()
        & 0o7777)
}

/// Sets the permission bits of the file at `path`.
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting the mode of {}", path.display()))
}

/// Never called: the runner is signed on macOS only.
#[cfg(not(unix))]
fn file_mode(_: &Path) -> anyhow::Result<u32> {
    bail!("the runner is signed on macOS")
}

/// Never called: the runner is signed on macOS only.
#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> anyhow::Result<()> {
    bail!("the runner is signed on macOS")
}

/// The entries of the archive at `path`.
fn list(path: &Path) -> anyhow::Result<Vec<String>> {
    Ok(entries(&cmd::stdout(
        Command::new(tool::TAR).arg("-tzf").arg(path),
    )?))
}

/// Signs, notarizes, repacks and uploads again every runner macOS archive on `tag`'s draft;
/// says so and does nothing when there is none.
///
/// # Errors
///
/// Fails when any archive can't be downloaded, signed, notarized, repacked as it was, or
/// uploaded.
pub(crate) fn sign_runners(plan: &BundlePlan, layout: &Layout, tag: &Tag) -> anyhow::Result<()> {
    let name = tag.name();
    let release = cmd::stdout(Command::new("gh").args([
        "release", "view", &name, "--json", "assets", "--repo", REPOSITORY,
    ]))?;
    let archives = runner_assets(&release, &tag.version());
    if archives.is_empty() {
        say(&format!(
            "No runner archives (emberhut-runner-{}-macos-*.tar.gz) on the draft; nothing more to sign",
            tag.version()
        ));
        return Ok(());
    }
    let Signing::Identity(identity) = &plan.signing else {
        bail!("the runner is signed with a Developer ID identity, not ad hoc");
    };
    let work = layout.out_dir().join("runner");
    private_dir(&work)?;
    for archive in archives {
        say(&format!("Signing the runner in {archive}"));
        let dir = work.join(archive.trim_end_matches(".tar.gz"));
        let (download, unpacked, out) =
            (dir.join("download"), dir.join("unpacked"), dir.join("out"));
        for path in [&download, &unpacked, &out] {
            private_dir(path)?;
        }
        cmd::run(
            Command::new("gh")
                .args(["release", "download", &name, "--pattern", &archive, "--dir"])
                .arg(&download)
                .args(["--clobber", "--repo", REPOSITORY]),
        )?;
        let original = download.join(&archive);
        let before = list(&original)?;
        let binary = unpacked.join(binary_entry(&before)?);
        cmd::run(
            Command::new(tool::TAR)
                .arg("-xpzf")
                .arg(&original)
                .arg("-C")
                .arg(&unpacked),
        )?;
        let mode = file_mode(&binary)?;
        cmd::run(&mut codesign_command(identity, &binary))?;
        // codesign rewrites the file; the archive keeps its mode (755) all the same.
        set_mode(&binary, mode)?;
        cmd::run(
            Command::new(tool::CODESIGN)
                .args(["--verify", "--strict", "--verbose=2"])
                .arg(&binary),
        )?;
        if let Some(notarization) = &plan.notarization {
            let upload = dir.join("notary-upload.zip");
            sign::zip_bundle(&binary, &upload)?;
            let verdict = sign::notarize_file(&upload, notarization, &dir.join("notary-log.txt"));
            remove_file_if_present(&upload)?;
            verdict?;
        }
        let repacked: PathBuf = out.join(&archive);
        cmd::run(&mut repack_command(&repacked, &unpacked, &before))?;
        let after = list(&repacked)?;
        ensure!(
            after == before,
            "{archive} repacked with other entries:\nbefore: {before:?}\nafter: {after:?}"
        );
        cmd::run(
            Command::new("gh")
                .args(["release", "upload", &name])
                .arg(&repacked)
                .args(["--clobber", "--repo", REPOSITORY]),
        )?;
        say(&format!("Signed, notarized and uploaded {archive} again"));
    }
    fs::remove_dir_all(&work).with_context(|| format!("removing {}", work.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{binary_entry, codesign_command, entries, repack_command, runner_assets};
    use crate::cmd::describe;

    #[test]
    fn only_this_versions_runner_archives_for_macos_are_picked() {
        let release = r#"{"assets":[
            {"name":"emberhut-runner-0.1.0-rc.1-macos-x64.tar.gz"},
            {"name":"emberhut-runner-0.1.0-rc.1-macos-arm64.tar.gz"},
            {"name":"emberhut-runner-0.1.0-rc.1-linux-x64.tar.gz"},
            {"name":"emberhut-runner-0.1.0-rc.10-macos-arm64.tar.gz"},
            {"name":"emberhut-runner-0.1.0-rc.1-macos-.tar.gz"},
            {"name":"emberhut-server-0.1.0-rc.1-macos-universal.tar.gz"}
        ]}"#;
        assert_eq!(
            runner_assets(release, "0.1.0-rc.1"),
            [
                "emberhut-runner-0.1.0-rc.1-macos-arm64.tar.gz",
                "emberhut-runner-0.1.0-rc.1-macos-x64.tar.gz"
            ]
        );
        assert_eq!(
            runner_assets(r#"{"assets":[]}"#, "0.1.0"),
            [] as [String; 0]
        );
        assert_eq!(runner_assets("not json", "0.1.0"), [] as [String; 0]);
    }

    fn listing() -> Vec<String> {
        entries(
            "emberhut-runner-0.1.0-macos-arm64/\n\
             emberhut-runner-0.1.0-macos-arm64/emberhut-runner\n\
             emberhut-runner-0.1.0-macos-arm64/LICENSE-MIT\n\
             emberhut-runner-0.1.0-macos-arm64/LICENSE-APACHE\n\
             emberhut-runner-0.1.0-macos-arm64/README.md\n",
        )
    }

    #[test]
    fn only_the_runner_binary_is_signed() {
        assert_eq!(
            binary_entry(&listing()).unwrap(),
            "emberhut-runner-0.1.0-macos-arm64/emberhut-runner"
        );
        let mut loose = listing();
        loose.push("elsewhere/emberhut-runner".into());
        assert!(binary_entry(&loose).is_err());
        let without: Vec<String> = listing()
            .into_iter()
            .filter(|entry| !entry.ends_with("/emberhut-runner"))
            .collect();
        assert!(binary_entry(&without).is_err());
    }

    /// The repack against real files: the layout CI uploads, packed by the same arguments,
    /// lists the same entries in the same order and keeps the binary's mode. Uses the system
    /// tar, as `release mac` does on the Mac.
    #[cfg(unix)]
    #[test]
    fn a_repacked_archive_keeps_its_entries_and_modes() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("unpacked");
        let top = root.join("emberhut-runner-0.1.0-macos-arm64");
        std::fs::create_dir_all(&top).unwrap();
        for (name, mode) in [
            ("emberhut-runner", 0o755),
            ("LICENSE-MIT", 0o644),
            ("LICENSE-APACHE", 0o644),
            ("README.md", 0o644),
        ] {
            let path = top.join(name);
            std::fs::write(&path, name).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let before = listing();
        let archive = dir.path().join("r.tar.gz");
        let repack = repack_command(&archive, &root, &before);
        // The test runs wherever tar is; `release mac` uses macOS's /usr/bin/tar.
        let mut command = Command::new("tar");
        command.args(repack.get_args()).envs(
            repack
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned()))),
        );
        assert!(command.status().unwrap().success());
        let listed = Command::new("tar")
            .arg("-tzf")
            .arg(&archive)
            .output()
            .unwrap();
        assert_eq!(entries(&String::from_utf8(listed.stdout).unwrap()), before);
        let verbose = Command::new("tar")
            .arg("-tvzf")
            .arg(&archive)
            .output()
            .unwrap();
        let verbose = String::from_utf8(verbose.stdout).unwrap();
        let runner = verbose
            .lines()
            .find(|line| line.ends_with("/emberhut-runner"))
            .unwrap();
        assert!(runner.starts_with("-rwxr-xr-x"), "{runner}");
    }

    #[test]
    fn the_repack_names_every_entry_in_order_without_recursing() {
        let before = listing();
        let command = repack_command(Path::new("/o/r.tar.gz"), Path::new("/u"), &before);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &args[..6],
            ["-cz", "--no-recursion", "-f", "/o/r.tar.gz", "-C", "/u"]
        );
        assert_eq!(
            &args[6..],
            before.as_slice(),
            "the same entries, in the same order"
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "COPYFILE_DISABLE" && value.is_some())
        );
        assert_eq!(entries(&(before.join("\n") + "\n\n")), before);
    }

    #[test]
    fn the_runner_is_signed_with_its_identifier_and_the_hardened_runtime() {
        assert_eq!(
            describe(&codesign_command(
                "Developer ID Application: Example (ABCDE12345)",
                Path::new("r/emberhut-runner")
            )),
            "/usr/bin/codesign --force --options runtime --timestamp --identifier \
             com.emberhut.runner --sign 'Developer ID Application: Example (ABCDE12345)' \
             r/emberhut-runner"
        );
    }
}
