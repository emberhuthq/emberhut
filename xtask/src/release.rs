//! `cargo xtask release`: the steps of a GitHub release (docs/releasing.md, ADR 0032).
//!
//! - `version <tag>` checks a tag and prints the numbers derived from it for `$GITHUB_ENV`.
//! - `notes` turns GitHub's generated release notes into the feed's plain notes.
//! - `feed` writes the updater's manifests, `SHA256SUMS` and their signatures with the update
//!   key; `verify-feed` reads them back as the app does.
//! - `keygen` makes a new update key.
//! - `mac` is the Mac's part: it builds, signs and notarizes the macOS assets on the Mac whose
//!   Keychain holds the Developer ID identity, uploads them to the draft release and starts the
//!   publish stage.

pub(crate) mod feed;
mod keygen;
mod mac;
pub(crate) mod notes;
mod runner;
pub(crate) mod tag;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, ensure};
use clap::{Args, Subcommand};
use emberhut_api::releases::Platform;

pub(crate) use self::mac::MacArgs;
use self::tag::Tag;
use crate::say;
use crate::workspace::Workspace;

/// The workspace's version, which every release tag's base must equal.
pub(crate) const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `cargo xtask release <step>`.
#[derive(Debug, Args)]
pub(crate) struct ReleaseArgs {
    /// The step.
    #[command(subcommand)]
    pub(crate) step: Step,
}

/// One step of a release.
#[derive(Debug, Subcommand)]
pub(crate) enum Step {
    /// Check a release tag and print `EMBERHUT_VERSION`, `EMBERHUT_BUILD`, `EMBERHUT_CHANNEL`,
    /// `EMBERHUT_PRERELEASE`, `EMBERHUT_MSI_VERSION` and `EMBERHUT_PREVIOUS_TAG` as `KEY=value`
    /// lines.
    ///
    /// The tag is `vX.Y.Z-rc.N` (N from 1 to 98) or `vX.Y.Z`, and `X.Y.Z` must be the workspace's
    /// version. The previous tag is the latest earlier release tag in this clone (empty without
    /// git or tags).
    Version {
        /// The tag, e.g. v0.1.0-rc.1.
        tag: String,
    },

    /// Turn the Markdown of GitHub's generated release notes into the feed's notes: one
    /// `- title (#N)` line per pull request, at most 4,000 bytes.
    Notes {
        /// The generated notes' body, as `gh api …/releases/generate-notes --jq .body` writes it.
        #[arg(long, value_name = "FILE")]
        github_body: PathBuf,
        /// Where to write the feed's notes.
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },

    /// Write the updater's feeds (`emberhut-desktop-macos.json`, `emberhut-desktop-windows.json`)
    /// for the desktop installers among the assets, then `SHA256SUMS` of every asset and feed,
    /// and `SHA256SUMS.minisig`, all signed with the update key.
    Feed {
        /// The release's tag.
        #[arg(long)]
        tag: String,
        /// The feed's notes, used byte for byte (see `release notes`).
        #[arg(long, value_name = "FILE")]
        notes_file: PathBuf,
        /// The update key: an unencrypted minisign secret key file. It is never printed.
        #[arg(long, value_name = "FILE")]
        key_file: PathBuf,
        /// Where the feeds and sums go.
        #[arg(long, value_name = "DIR")]
        out_dir: PathBuf,
        /// Every other asset of the release.
        #[arg(required = true, value_name = "ASSET")]
        assets: Vec<PathBuf>,
    },

    /// Check the feeds and sums `release feed` wrote, as the app would: shape, URLs, sizes,
    /// hashes, and every signature with the given public key.
    VerifyFeed {
        /// The release's tag.
        #[arg(long)]
        tag: String,
        /// The folder holding the feeds, the sums and the assets.
        #[arg(long, value_name = "DIR")]
        dir: PathBuf,
        /// The update key's public half, base64 (the second line of its `.pub` file).
        #[arg(long, value_name = "BASE64")]
        public_key: String,
        /// A platform whose feed must be there (`macos`, `windows`); repeat for each.
        #[arg(long, value_name = "PLATFORM")]
        expect: Vec<String>,
    },

    /// Make a new update key: a minisign key pair, the secret half written unencrypted to
    /// `<name>.key` (mode 0600), the public half to `<name>.pub`. Prints the key id and the
    /// public key, never the secret.
    Keygen {
        /// Where to write the two files; neither may exist.
        #[arg(long, value_name = "DIR")]
        out_dir: PathBuf,
        /// The files' name, without the extension.
        #[arg(long, default_value = "emberhut-update")]
        name: String,
    },

    /// The Mac's part of a release: after checking the tag, CI's tag run and the draft, build
    /// the universal app and server, sign them with the Keychain's Developer ID identity,
    /// notarize them, upload `Emberhut-<version>-macos-universal.dmg` and
    /// `emberhut-server-<version>-macos-universal.tar.gz` to the draft release, sign and notarize
    /// the runner binaries in the draft's `emberhut-runner-<version>-macos-*.tar.gz` and upload
    /// them again, and start the release workflow's publish stage. Runs on macOS only.
    Mac(MacArgs),
}

/// Runs `release` with `args`.
///
/// # Errors
///
/// Fails when the step fails.
pub(crate) fn run(args: &ReleaseArgs) -> anyhow::Result<()> {
    match &args.step {
        Step::Version { tag } => {
            let tag = release_tag(tag)?;
            let previous = previous_tag(&tag);
            emit(&tag::env_lines(&tag, previous.as_ref()));
            Ok(())
        }
        Step::Notes { github_body, out } => {
            let body = fs::read_to_string(github_body)
                .with_context(|| format!("reading {}", github_body.display()))?;
            let notes = notes::feed_notes(&body);
            fs::write(out, &notes).with_context(|| format!("writing {}", out.display()))?;
            say(&format!(
                "Wrote {} ({} lines)",
                out.display(),
                notes.lines().count()
            ));
            Ok(())
        }
        Step::Feed {
            tag,
            notes_file,
            key_file,
            out_dir,
            assets,
        } => {
            let tag = release_tag(tag)?;
            let notes = fs::read_to_string(notes_file)
                .with_context(|| format!("reading {}", notes_file.display()))?;
            let key = feed::UpdateKey::read(key_file)?;
            let input = feed::FeedInput {
                tag: &tag,
                notes: &notes,
                assets,
                moment: feed::Moment::now(),
            };
            feed::write(&input, &key, out_dir).map(drop)
        }
        Step::VerifyFeed {
            tag,
            dir,
            public_key,
            expect,
        } => {
            let tag = release_tag(tag)?;
            let expect = expect
                .iter()
                .map(|name| {
                    let platform = Platform::from(name.as_str());
                    ensure!(
                        matches!(platform, Platform::Macos | Platform::Windows),
                        "--expect takes macos or windows, not {name}"
                    );
                    Ok(platform)
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let checked = feed::verify(dir, &tag, public_key, &expect)?;
            say(&format!(
                "The feeds and sums of {} verify ({checked} installer signatures and SHA256SUMS)",
                tag.name()
            ));
            Ok(())
        }
        Step::Keygen { out_dir, name } => {
            let (key, files) = keygen::keygen(out_dir, name)?;
            say(&format!(
                "Wrote the secret key to {} (mode 0600; keep it out of any repository and log) \
                 and the public key to {}",
                files.secret.display(),
                files.public.display()
            ));
            emit(&[
                format!("key id: {}", key.id()),
                format!("public key: {}", key.public_base64()),
            ]);
            Ok(())
        }
        Step::Mac(args) => mac::run(args),
    }
}

/// `tag` read as a release tag of this workspace.
///
/// # Errors
///
/// Fails when it isn't a release tag or its base isn't the workspace's version.
pub(crate) fn release_tag(tag: &str) -> anyhow::Result<Tag> {
    let parsed = Tag::parse(tag)?;
    ensure!(
        parsed.base() == WORKSPACE_VERSION,
        "{tag} is a release of {}, but the workspace's version is {WORKSPACE_VERSION}; tag a \
         commit whose Cargo.toml has version = \"{}\"",
        parsed.base(),
        parsed.base()
    );
    Ok(parsed)
}

/// The latest release tag before `tag` in this clone, if git can list the tags.
fn previous_tag(tag: &Tag) -> Option<Tag> {
    let root = Workspace::root_dir();
    let tags = git_tags(&root);
    if tags.is_none() {
        say("git couldn't list the tags; EMBERHUT_PREVIOUS_TAG is empty");
    }
    tag::previous(tag, tags.as_deref().unwrap_or_default().lines())
}

/// `git tag --list 'v*'` in `root`, or `None` when git isn't there or fails.
fn git_tags(root: &Path) -> Option<String> {
    let out = crate::cmd::capture(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["tag", "--list", "v*"]),
    )
    .ok()?;
    out.status.success().then_some(out.stdout)
}

/// Prints `lines`, the step's output for a script to read.
#[expect(
    clippy::print_stdout,
    reason = "these lines are the step's output, read by the release workflow"
)]
fn emit(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::{WORKSPACE_VERSION, release_tag};

    #[test]
    fn a_tag_must_release_the_workspaces_version() {
        let tag = release_tag(&format!("v{WORKSPACE_VERSION}-rc.1")).unwrap();
        assert_eq!(tag.base(), WORKSPACE_VERSION);
        let err = release_tag("v99.0.0").unwrap_err().to_string();
        assert!(err.contains("the workspace's version is"), "{err}");
        assert!(release_tag("0.1.0").is_err());
    }
}
