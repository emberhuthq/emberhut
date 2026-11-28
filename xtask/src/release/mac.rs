//! `release mac`: the Mac's part of a release. The Developer ID identity never leaves that Mac's
//! Keychain and no Apple credential is a CI secret (ADR 0032), so the macOS assets are built,
//! signed and notarized here, after CI has built the rest and opened the draft release:
//!
//! 1. the checks: a clean tree at exactly the tag, the tag as on `origin`, the release
//!    workflow's tag run green, and the draft release there;
//! 2. `bundle-macos --dist --universal --dmg --sign … --notarize` with the tag's version and
//!    build, under `nice`;
//! 3. the universal server: signed with the hardened runtime and a timestamp, notarized (as a
//!    zip: a bare binary can't be stapled) and packed with the licenses and README;
//! 4. Gatekeeper's checks and the app's `--self-check` smoke run, from the disk image, when a
//!    desktop session is there for them;
//! 5. both assets uploaded to the draft, and the workflow's publish stage started.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail, ensure};
use clap::{ArgGroup, Args};

use crate::bundle_macos::plan::{BundlePlan, BundleVersion};
use crate::bundle_macos::{self, BundleArgs, Layout, dmg, sign, tool};
use crate::cmd;
use crate::files::{remove_dir_if_present, remove_file_if_present};
use crate::release::feed::REPOSITORY;
use crate::release::release_tag;
use crate::release::tag::Tag;
use crate::say;
use crate::workspace::Workspace;

/// The release workflow, as `gh` names it.
pub(crate) const WORKFLOW: &str = "release.yml";
/// The files every server archive holds beside the binary.
const SERVER_EXTRAS: [&str; 3] = ["LICENSE-MIT", "LICENSE-APACHE", "README.md"];

/// Options of `cargo xtask release mac`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(group(ArgGroup::new("notary").required(true).args(["keychain_profile", "notary_key"])))]
pub(crate) struct MacArgs {
    /// The release tag, e.g. v0.1.0-rc.1. The checkout must be exactly at it.
    #[arg(long)]
    pub(crate) tag: String,

    /// The Developer ID Application identity in this Mac's Keychain, as
    /// `security find-identity -v -p codesigning` lists it.
    #[arg(long, value_name = "IDENTITY", env = "EMBERHUT_SIGN_IDENTITY")]
    pub(crate) sign: String,

    /// The notarytool keychain profile with the App Store Connect credentials.
    #[arg(long, value_name = "NAME", env = "EMBERHUT_NOTARY_PROFILE")]
    pub(crate) keychain_profile: Option<String>,

    /// Notarize with this App Store Connect API key (`AuthKey_<id>.p8`), read in place.
    #[arg(
        long,
        value_name = "PATH",
        env = "EMBERHUT_NOTARY_KEY",
        conflicts_with = "keychain_profile",
        requires_all = ["notary_key_id", "notary_issuer"]
    )]
    pub(crate) notary_key: Option<PathBuf>,

    /// The API key's id, with --notary-key.
    #[arg(long, value_name = "ID", env = "EMBERHUT_NOTARY_KEY_ID")]
    pub(crate) notary_key_id: Option<String>,

    /// The API key's issuer id, with --notary-key.
    #[arg(long, value_name = "UUID", env = "EMBERHUT_NOTARY_ISSUER")]
    pub(crate) notary_issuer: Option<String>,

    /// How many compile jobs cargo may run at once (the build runs under `nice`).
    #[arg(
        long,
        short = 'j',
        value_name = "N",
        default_value_t = 4,
        value_parser = clap::value_parser!(u16).range(1..)
    )]
    pub(crate) jobs: u16,

    /// Upload to the draft but don't start the publish stage.
    #[arg(long)]
    pub(crate) no_publish: bool,
}

impl MacArgs {
    /// The `bundle-macos` plan of the release build of `tag`.
    ///
    /// # Errors
    ///
    /// Fails when the identity or the notary credentials don't make a notarized build.
    pub(crate) fn plan(&self, tag: &Tag) -> anyhow::Result<BundlePlan> {
        let mut plan = BundleArgs {
            release: false,
            dist: true,
            automation: false,
            jobs: self.jobs,
            sign: Some(self.sign.clone()),
            notarize: true,
            keychain_profile: self.keychain_profile.clone(),
            notary_key: self.notary_key.clone(),
            notary_key_id: self.notary_key_id.clone(),
            notary_issuer: self.notary_issuer.clone(),
            universal: true,
            dmg: true,
            no_hut: false,
            install: false,
        }
        .plan()?;
        plan.version = BundleVersion::release(tag);
        Ok(plan)
    }
}

/// The macOS server archive's name, and the folder inside it.
#[must_use]
pub(crate) fn server_archive(version: &str) -> (String, String) {
    let dir = format!("emberhut-server-{version}-macos-universal");
    (format!("{dir}.tar.gz"), dir)
}

/// The commit `git ls-remote origin refs/tags/<tag>*` says `tag` is on: the peeled `^{}` line
/// of an annotated tag, else the tag's own.
#[must_use]
pub(crate) fn remote_tag_commit(ls_remote: &str, tag: &str) -> Option<String> {
    let reference = format!("refs/tags/{tag}");
    let peeled = format!("{reference}^{{}}");
    let lines: Vec<(&str, &str)> = ls_remote
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    lines
        .iter()
        .find(|(_, name)| *name == peeled)
        .or_else(|| lines.iter().find(|(_, name)| *name == reference))
        .map(|(sha, _)| (*sha).to_owned())
}

/// The title release.yml's `run-name` gives the build stage's run of `tag` (the tag's push, or
/// a `stage=build` rerun).
#[must_use]
pub(crate) fn build_run_title(tag: &str) -> String {
    format!("Release {tag} (build)")
}

/// Whether `gh run list --json headBranch,displayTitle,status,conclusion` output has a
/// successful build-stage run of the release workflow for `tag`.
#[must_use]
pub(crate) fn tag_run_succeeded(runs_json: &str, tag: &str) -> bool {
    let Ok(serde_json::Value::Array(runs)) = serde_json::from_str(runs_json) else {
        return false;
    };
    let title = build_run_title(tag);
    runs.iter().any(|run| {
        run["headBranch"] == tag
            && run["displayTitle"] == title.as_str()
            && run["status"] == "completed"
            && run["conclusion"] == "success"
    })
}

/// What's wrong with the release `gh release view --json tagName,isDraft,isPrerelease` shows
/// for `tag`, if anything.
#[must_use]
pub(crate) fn draft_problem(release_json: &str, tag: &Tag) -> Option<String> {
    let Ok(release) = serde_json::from_str::<serde_json::Value>(release_json) else {
        return Some("gh's answer about the release doesn't read".to_owned());
    };
    let name = tag.name();
    if release["tagName"] != name.as_str() {
        return Some(format!("the release isn't {name}'s"));
    }
    if release["isDraft"] != true {
        return Some(format!(
            "{name}'s release is published already; a published release isn't changed"
        ));
    }
    if release["isPrerelease"] != tag.prerelease() {
        return Some(format!(
            "{name}'s draft should be marked {}",
            if tag.prerelease() {
                "a pre-release"
            } else {
                "a full release"
            }
        ));
    }
    None
}

/// A `gh` command on the repository.
fn gh(args: &[&str]) -> Command {
    let mut command = Command::new("gh");
    command.args(args).args(["--repo", REPOSITORY]);
    command
}

/// The `gh run list` command for the release workflow's runs of `commit`.
#[must_use]
pub(crate) fn run_list_command(commit: &str) -> Command {
    gh(&[
        "run",
        "list",
        "--workflow",
        WORKFLOW,
        "--commit",
        commit,
        "--json",
        "headBranch,displayTitle,status,conclusion,url",
        "--limit",
        "50",
    ])
}

/// The `gh release view` command for `tag`.
#[must_use]
pub(crate) fn release_view_command(tag: &Tag) -> Command {
    gh(&[
        "release",
        "view",
        &tag.name(),
        "--json",
        "tagName,isDraft,isPrerelease",
    ])
}

/// The `gh release upload --clobber` command for `files`.
#[must_use]
pub(crate) fn upload_command(tag: &Tag, files: &[PathBuf]) -> Command {
    let mut command = Command::new("gh");
    command
        .args(["release", "upload", &tag.name()])
        .args(files)
        .args(["--clobber", "--repo", REPOSITORY]);
    command
}

/// The `gh workflow run` command that starts the publish stage of `tag`, on the tag's ref so the
/// tag-only `release` environment admits it.
#[must_use]
pub(crate) fn publish_command(tag: &Tag) -> Command {
    let name = tag.name();
    gh(&[
        "workflow",
        "run",
        WORKFLOW,
        "--ref",
        &name,
        "-f",
        &format!("tag={name}"),
        "-f",
        "stage=publish",
    ])
}

/// A git command in `root`.
fn git(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    command
}

/// Runs `release mac`.
///
/// # Errors
///
/// Fails when a check fails or any step does; nothing is uploaded before every asset is built,
/// signed and notarized.
pub(crate) fn run(args: &MacArgs) -> anyhow::Result<()> {
    bundle_macos::require_macos("release mac")?;
    let tag = release_tag(&args.tag)?;
    let plan = args.plan(&tag)?;
    let layout = Layout::new(Workspace::discover()?);
    check(&tag, layout.root())?;

    let bundled = bundle_macos::bundle(&plan, &layout)?;
    let dmg = bundled
        .dmg
        .context("bundle-macos made no disk image with --dmg")?;
    let server = bundled
        .server
        .context("bundle-macos built no server without --no-hut")?;
    let archive = package_server(&plan, &layout, &server, &tag)?;
    smoke(&layout, &dmg)?;

    say(&format!("Uploading to the draft release {}", tag.name()));
    cmd::run(&mut upload_command(&tag, &[dmg, archive]))?;
    if args.no_publish {
        say(&format!(
            "Uploaded; start the publish stage with `{}`",
            cmd::describe(&publish_command(&tag))
        ));
    } else {
        cmd::run(&mut publish_command(&tag))?;
        say(&format!(
            "Started the publish stage of {}; follow it with `gh run list --workflow {WORKFLOW}`",
            tag.name()
        ));
    }
    Ok(())
}

/// The checks before anything is built.
fn check(tag: &Tag, root: &Path) -> anyhow::Result<()> {
    let name = tag.name();
    let status = cmd::stdout(&mut git(root, &["status", "--porcelain"]))?;
    ensure!(
        status.trim().is_empty(),
        "the checkout has changes; release from a clean checkout of {name}:\n{status}"
    );
    let head = cmd::stdout(&mut git(root, &["rev-parse", "HEAD"]))?;
    let tagged = cmd::stdout(&mut git(
        root,
        &["rev-parse", &format!("{name}^{{commit}}")],
    ))
    .with_context(|| format!("there's no tag {name} here; `git fetch --tags` first"))?;
    ensure!(
        head.trim() == tagged.trim(),
        "HEAD isn't {name}; `git switch --detach {name}` first"
    );
    let remote = cmd::stdout(&mut git(
        root,
        &[
            "ls-remote",
            "origin",
            &format!("refs/tags/{name}"),
            &format!("refs/tags/{name}^{{}}"),
        ],
    ))?;
    ensure!(
        remote_tag_commit(&remote, &name).as_deref() == Some(tagged.trim()),
        "{name} on origin isn't this commit"
    );
    let runs = cmd::stdout(&mut run_list_command(tagged.trim()))?;
    ensure!(
        tag_run_succeeded(&runs, &name),
        "the release workflow's build run for {name} (\"{}\") hasn't succeeded (yet); see \
         `gh run list --workflow {WORKFLOW}`",
        build_run_title(&name)
    );
    let release = cmd::stdout(&mut release_view_command(tag))
        .with_context(|| format!("there's no release for {name}; the tag run makes the draft"))?;
    if let Some(problem) = draft_problem(&release, tag) {
        bail!(problem);
    }
    say(&format!(
        "{name} is clean, on origin, built by CI, and its draft is waiting"
    ));
    Ok(())
}

/// Signs, notarizes and packs the universal server `binary` as the release's macOS server
/// archive; returns its path.
fn package_server(
    plan: &BundlePlan,
    layout: &Layout,
    binary: &Path,
    tag: &Tag,
) -> anyhow::Result<PathBuf> {
    let (archive_name, dir_name) = server_archive(&tag.version());
    let work = layout.server_dir();
    remove_dir_if_present(&work)?;
    let dir = work.join(&dir_name);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let server = dir.join("emberhut-server");
    fs::copy(binary, &server)
        .with_context(|| format!("copying {} to {}", binary.display(), server.display()))?;
    for extra in SERVER_EXTRAS {
        let from = layout.root().join(extra);
        fs::copy(&from, dir.join(extra)).with_context(|| format!("copying {}", from.display()))?;
    }
    sign::sign(&plan.signing, &server, None)?;
    if let Some(notarization) = &plan.notarization {
        let upload = work.join("server-notary-upload.zip");
        sign::zip_bundle(&server, &upload)?;
        let verdict = sign::notarize_file(&upload, notarization, &work.join("notary-log.txt"));
        remove_file_if_present(&upload)?;
        verdict?;
        say("Notarized the server (a bare binary can't be stapled; Gatekeeper checks it online)");
    }
    let archive = layout.out_dir().join(&archive_name);
    remove_file_if_present(&archive)?;
    cmd::run(
        Command::new(tool::TAR)
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&work)
            .arg(&dir_name),
    )?;
    say(&format!("Packed {}", archive.display()));
    Ok(archive)
}

/// The app's smoke run from the disk image, as a person would open it, when a desktop session
/// is there to draw in.
fn smoke(layout: &Layout, dmg: &Path) -> anyhow::Result<()> {
    if !dmg::gui_session() {
        say("Skipped the --self-check smoke run: no logged-in desktop session");
        return Ok(());
    }
    let mount = layout.out_dir().join("dmg-mount");
    remove_dir_if_present(&mount)?;
    fs::create_dir_all(&mount).with_context(|| format!("creating {}", mount.display()))?;
    cmd::run(&mut dmg::attach_command(dmg, &mount))?;
    say("Running the app from the disk image with --self-check");
    let ran = cmd::run(
        Command::new(
            mount
                .join("Emberhut.app")
                .join("Contents")
                .join("MacOS")
                .join("emberhut-desktop"),
        )
        .args(["--self-check", "--background"]),
    );
    let detached = cmd::run(&mut dmg::detach_command(&mount));
    ran.context("the app from the disk image failed its smoke run")?;
    detached?;
    say("The app from the disk image drew its first frame");
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory as _, FromArgMatches as _};

    use super::{
        MacArgs, draft_problem, publish_command, remote_tag_commit, run_list_command,
        server_archive, tag_run_succeeded, upload_command,
    };
    use crate::bundle_macos::plan::{Notarization, Signing};
    use crate::cli::{Cli, Task};
    use crate::cmd::describe;
    use crate::release::tag::Tag;
    use crate::release::{ReleaseArgs, Step};

    fn tag(name: &str) -> Tag {
        Tag::parse(name).unwrap()
    }

    fn parse(args: &[&str]) -> Result<MacArgs, clap::Error> {
        let command = Cli::command().mut_subcommand("release", |release| {
            release.mut_subcommand("mac", |mac| {
                [
                    "sign",
                    "keychain_profile",
                    "notary_key",
                    "notary_key_id",
                    "notary_issuer",
                ]
                .into_iter()
                .fold(mac, |mac, id| mac.mut_arg(id, |arg| arg.env(None::<&str>)))
            })
        });
        let matches =
            command.try_get_matches_from(["cargo xtask", "release", "mac"].iter().chain(args))?;
        match Cli::from_arg_matches(&matches)?.task {
            Task::Release(ReleaseArgs {
                step: Step::Mac(mac),
            }) => Ok(mac),
            _ => panic!("release mac"),
        }
    }

    #[test]
    fn the_mac_leg_plans_a_notarized_universal_disk_image() {
        let args = parse(&[
            "--tag",
            "v0.1.0-rc.1",
            "--sign",
            "Developer ID Application: Example (ABCDE12345)",
            "--keychain-profile",
            "emberhut-notary",
        ])
        .unwrap();
        assert_eq!(args.jobs, 4);
        assert!(!args.no_publish);
        let plan = args.plan(&tag("v0.1.0-rc.1")).unwrap();
        assert!(plan.universal && plan.dmg && plan.hut && !plan.install);
        assert_eq!(plan.profile.name(), "dist");
        assert_eq!(
            plan.signing,
            Signing::Identity("Developer ID Application: Example (ABCDE12345)".into())
        );
        assert_eq!(
            plan.notarization,
            Some(Notarization::Profile {
                keychain_profile: "emberhut-notary".into()
            })
        );
        assert_eq!(plan.version.bundle, "10001");
        assert_eq!(plan.version.full, "0.1.0-rc.1");
    }

    #[test]
    fn the_mac_leg_needs_an_identity_and_notary_credentials() {
        let err = parse(&["--tag", "v0.1.0", "--keychain-profile", "p"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        let err = parse(&["--tag", "v0.1.0", "--sign", "Example"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        let args = parse(&[
            "--tag",
            "v0.1.0",
            "--sign",
            "-",
            "--notary-key",
            "/keys/AuthKey_ABC123.p8",
            "--notary-key-id",
            "ABC123",
            "--notary-issuer",
            "issuer",
            "--no-publish",
            "-j",
            "2",
        ])
        .unwrap();
        assert!(args.no_publish);
        assert!(
            args.plan(&tag("v0.1.0")).is_err(),
            "an ad hoc signature can't be notarized"
        );
    }

    #[test]
    fn the_server_archive_is_named_for_the_version() {
        assert_eq!(
            server_archive("0.1.0-rc.1"),
            (
                "emberhut-server-0.1.0-rc.1-macos-universal.tar.gz".to_owned(),
                "emberhut-server-0.1.0-rc.1-macos-universal".to_owned()
            )
        );
    }

    #[test]
    fn the_remote_tag_is_its_peeled_commit() {
        let annotated = "1111111111111111111111111111111111111111\trefs/tags/v0.1.0\n\
                         2222222222222222222222222222222222222222\trefs/tags/v0.1.0^{}\n";
        assert_eq!(
            remote_tag_commit(annotated, "v0.1.0").as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        let light = "3333333333333333333333333333333333333333\trefs/tags/v0.1.0\n";
        assert_eq!(
            remote_tag_commit(light, "v0.1.0").as_deref(),
            Some("3333333333333333333333333333333333333333")
        );
        assert_eq!(remote_tag_commit(light, "v0.1.0-rc.1"), None);
        assert_eq!(remote_tag_commit("", "v0.1.0"), None);
    }

    #[test]
    fn only_a_green_build_run_of_the_tag_counts() {
        let runs = r#"[
            {"headBranch":"v0.1.0-rc.1","displayTitle":"Release v0.1.0-rc.1 (publish)","status":"completed","conclusion":"success"},
            {"headBranch":"v0.1.0-rc.1","displayTitle":"Release v0.1.0-rc.1 (build)","status":"completed","conclusion":"failure"},
            {"headBranch":"v0.1.0-rc.1","displayTitle":"Release v0.1.0-rc.1 (build)","status":"in_progress","conclusion":""},
            {"headBranch":"main","displayTitle":"Release v0.1.0-rc.1 (build)","status":"completed","conclusion":"success"}
        ]"#;
        assert!(!tag_run_succeeded(runs, "v0.1.0-rc.1"));
        let green = r#"[{"headBranch":"v0.1.0-rc.1","displayTitle":"Release v0.1.0-rc.1 (build)","status":"completed","conclusion":"success"}]"#;
        assert!(tag_run_succeeded(green, "v0.1.0-rc.1"));
        assert!(!tag_run_succeeded(green, "v0.1.0"));
        assert!(!tag_run_succeeded("not json", "v0.1.0"));
    }

    #[test]
    fn the_release_must_be_a_draft_of_the_right_kind() {
        let rc = tag("v0.1.0-rc.1");
        let draft = r#"{"tagName":"v0.1.0-rc.1","isDraft":true,"isPrerelease":true}"#;
        assert_eq!(draft_problem(draft, &rc), None);
        let published = r#"{"tagName":"v0.1.0-rc.1","isDraft":false,"isPrerelease":true}"#;
        assert!(
            draft_problem(published, &rc)
                .unwrap()
                .contains("published already")
        );
        let full = r#"{"tagName":"v0.1.0-rc.1","isDraft":true,"isPrerelease":false}"#;
        assert!(draft_problem(full, &rc).unwrap().contains("a pre-release"));
        assert!(draft_problem(draft, &tag("v0.1.0")).is_some());
        assert!(draft_problem("", &rc).is_some());
    }

    #[test]
    fn the_gh_commands_name_the_repository_and_the_tag() {
        let rc = tag("v0.1.0-rc.1");
        assert_eq!(
            describe(&run_list_command("abc123")),
            "gh run list --workflow release.yml --commit abc123 --json \
             headBranch,displayTitle,status,conclusion,url --limit 50 --repo emberhuthq/emberhut"
        );
        assert_eq!(
            describe(&upload_command(&rc, &["a.dmg".into(), "b.tar.gz".into()])),
            "gh release upload v0.1.0-rc.1 a.dmg b.tar.gz --clobber --repo emberhuthq/emberhut"
        );
        assert_eq!(
            describe(&publish_command(&rc)),
            "gh workflow run release.yml --ref v0.1.0-rc.1 -f tag=v0.1.0-rc.1 -f stage=publish \
             --repo emberhuthq/emberhut"
        );
    }
}
