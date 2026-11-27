//! Installing to /Applications/Emberhut.app, and never anywhere else under /Applications.
//!
//! The new copy is written next to the old one first (`.Emberhut.app.new`, same volume), the old
//! one is renamed aside (`.Emberhut.app.old`), the new one renamed into place, and only then is
//! the old one removed. A failure on the way puts everything back as it was.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

use crate::bundle_macos::app::BUNDLE_DIR;
use crate::bundle_macos::ownership::{holds_ours, read_identifier};
use crate::bundle_macos::tool;
use crate::{cmd, say};

/// The directory `--install` writes into, and only at [`InstallPaths`] in it.
const APPLICATIONS: &str = "/Applications";

/// The three paths an install touches in its directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallPaths {
    /// Where the app ends up: `<dir>/Emberhut.app`.
    pub(crate) dest: PathBuf,
    /// Where the new copy is written first: `<dir>/.Emberhut.app.new`.
    pub(crate) new: PathBuf,
    /// Where the old app waits until the new one is in place: `<dir>/.Emberhut.app.old`.
    pub(crate) old: PathBuf,
}

impl InstallPaths {
    /// The paths of an install into `dir`.
    #[must_use]
    pub(crate) fn in_dir(dir: &Path) -> Self {
        Self {
            dest: dir.join(BUNDLE_DIR),
            new: dir.join(format!(".{BUNDLE_DIR}.new")),
            old: dir.join(format!(".{BUNDLE_DIR}.old")),
        }
    }
}

/// Which of the [`InstallPaths`] already hold a copy of this app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Occupancy {
    /// An installed Emberhut.app.
    pub(crate) dest: bool,
    /// A `.new` copy left by an earlier failed install.
    pub(crate) stale_new: bool,
    /// An `.old` copy left by an earlier failed install.
    pub(crate) stale_old: bool,
}

/// One step of an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Remove a `.new` copy an earlier run left.
    RemoveStaleNew,
    /// Remove an `.old` copy an earlier run left next to an installed app.
    RemoveStaleOld,
    /// Rename an `.old` copy an earlier run left back to the free destination: it may be the
    /// only copy of the app, so it is kept until the new one is in place.
    RestoreOld,
    /// Copy the bundle to `.new`.
    CopyToNew,
    /// Rename the installed app to `.old`.
    MoveOldAside,
    /// Rename `.new` to the destination.
    MoveNewIntoPlace,
    /// Remove `.old`, now that the new app is in place.
    RemoveOld,
}

/// The steps of an install, given what is already there.
///
/// An `.old` copy with no installed app beside it is what a failed rollback leaves: the only
/// copy of the app. It goes back into place first, and the install then replaces it as usual.
#[must_use]
pub(crate) fn steps(occupancy: Occupancy) -> Vec<Step> {
    let mut steps = Vec::new();
    if occupancy.stale_new {
        steps.push(Step::RemoveStaleNew);
    }
    let installed = match (occupancy.dest, occupancy.stale_old) {
        (true, true) => {
            steps.push(Step::RemoveStaleOld);
            true
        }
        (false, true) => {
            steps.push(Step::RestoreOld);
            true
        }
        (dest, false) => dest,
    };
    steps.push(Step::CopyToNew);
    if installed {
        steps.push(Step::MoveOldAside);
    }
    steps.push(Step::MoveNewIntoPlace);
    if installed {
        steps.push(Step::RemoveOld);
    }
    steps
}

/// What an install needs from the system; tests stand in for plutil and ditto.
pub(crate) trait System {
    /// The `CFBundleIdentifier` in the plist at `info_plist`, `None` when it can't be read.
    ///
    /// # Errors
    ///
    /// Fails when the plist can't be looked at at all.
    fn identifier(&self, info_plist: &Path) -> anyhow::Result<Option<String>>;

    /// Copies the bundle at `from` to the free path `to`.
    ///
    /// # Errors
    ///
    /// Fails when the copy fails; `to` may then hold part of it.
    fn copy_bundle(&self, from: &Path, to: &Path) -> anyhow::Result<()>;
}

/// The real thing: plutil reads bundle ids, ditto copies.
#[derive(Debug, Clone, Copy)]
struct MacOs;

impl System for MacOs {
    fn identifier(&self, info_plist: &Path) -> anyhow::Result<Option<String>> {
        read_identifier(info_plist)
    }

    fn copy_bundle(&self, from: &Path, to: &Path) -> anyhow::Result<()> {
        cmd::run(Command::new(tool::DITTO).arg(from).arg(to))
    }
}

/// Checks, before anything is built, that an install into /Applications could go ahead.
///
/// # Errors
///
/// Fails when any of the [`InstallPaths`] holds something that is not this app.
pub(crate) fn preflight() -> anyhow::Result<()> {
    survey(&InstallPaths::in_dir(Path::new(APPLICATIONS)), &MacOs).map(drop)
}

/// Installs `bundle` as /Applications/Emberhut.app and returns that path.
///
/// # Errors
///
/// Fails when [`preflight`] would, or a step fails (after undoing the steps before it).
pub(crate) fn install(bundle: &Path) -> anyhow::Result<PathBuf> {
    install_into(Path::new(APPLICATIONS), bundle, &MacOs)
}

/// Installs `bundle` as `<dir>/Emberhut.app`, using `system` for bundle ids and copying.
fn install_into(dir: &Path, bundle: &Path, system: &dyn System) -> anyhow::Result<PathBuf> {
    let paths = InstallPaths::in_dir(dir);
    let occupancy = survey(&paths, system)?;
    run_steps(&steps(occupancy), &paths, bundle, system)?;
    Ok(paths.dest)
}

/// What is at each install path, refusing anything that is not this app.
fn survey(paths: &InstallPaths, system: &dyn System) -> anyhow::Result<Occupancy> {
    let identify = |info: &Path| system.identifier(info);
    Ok(Occupancy {
        dest: holds_ours(&paths.dest, identify)?,
        stale_new: holds_ours(&paths.new, identify)?,
        stale_old: holds_ours(&paths.old, identify)?,
    })
}

/// Runs `steps` in order; on a failure, undoes what was done and returns the error.
fn run_steps(
    steps: &[Step],
    paths: &InstallPaths,
    bundle: &Path,
    system: &dyn System,
) -> anyhow::Result<()> {
    let mut moved_aside = false;
    for &step in steps {
        let done = match step {
            Step::RemoveStaleNew => remove(&paths.new),
            Step::RemoveStaleOld => remove(&paths.old),
            Step::RestoreOld => rename(&paths.old, &paths.dest),
            Step::CopyToNew => system.copy_bundle(bundle, &paths.new),
            Step::MoveOldAside => rename(&paths.dest, &paths.old),
            Step::MoveNewIntoPlace => rename(&paths.new, &paths.dest),
            Step::RemoveOld => {
                // The new app is in place: a leftover .old is untidy, not a failed install.
                if let Err(err) = remove(&paths.old) {
                    say(&format!("warning: {err:#}; remove it yourself"));
                }
                Ok(())
            }
        };
        match done {
            Ok(()) => moved_aside |= step == Step::MoveOldAside,
            Err(err) => return Err(roll_back(paths, moved_aside, err)),
        }
    }
    Ok(())
}

/// Undoes a failed install: removes the `.new` copy (ours: it was free or checked to be ours)
/// and, if the old app was moved aside, moves it back.
fn roll_back(paths: &InstallPaths, moved_aside: bool, err: anyhow::Error) -> anyhow::Error {
    let mut problems = Vec::new();
    if fs::symlink_metadata(&paths.new).is_ok()
        && let Err(undo) = remove(&paths.new)
    {
        problems.push(format!("{undo:#}"));
    }
    if moved_aside && let Err(undo) = rename(&paths.old, &paths.dest) {
        problems.push(format!("{undo:#}"));
    }
    if problems.is_empty() {
        err.context(format!(
            "installing failed; {} holds the app installed before, if there was one",
            paths.dest.display()
        ))
    } else {
        err.context(format!(
            "installing failed, and undoing it failed too ({}); check {} by hand",
            problems.join("; "),
            paths.dest.display()
        ))
    }
}

/// Removes the directory tree at `path`.
fn remove(path: &Path) -> anyhow::Result<()> {
    fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
}

/// Renames `from` to `to`.
fn rename(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::rename(from, to).with_context(|| format!("moving {} to {}", from.display(), to.display()))
}

#[cfg(test)]
mod tests;
