//! Whether something at an install path is this app, and so ours to replace or remove.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

use crate::bundle_macos::app::BUNDLE_ID;
use crate::bundle_macos::tool;
use crate::cmd;

/// Why an existing bundle must be left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Its `Info.plist` has no readable `CFBundleIdentifier`.
    UnknownIdentifier,
    /// It is another app.
    OtherApp {
        /// The identifier it has.
        identifier: String,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownIdentifier => {
                write!(f, "its Info.plist has no readable CFBundleIdentifier")
            }
            Self::OtherApp { identifier } => {
                write!(f, "it is another app ({identifier}, not {BUNDLE_ID})")
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// Whether a bundle whose `CFBundleIdentifier` reads `identifier` (`None`: unreadable) may be
/// replaced or removed: only when it is this app.
///
/// # Errors
///
/// Returns the [`Refusal`] when the bundle is another app or its identifier is unknown.
pub(crate) fn may_replace(identifier: Option<&str>) -> Result<(), Refusal> {
    match identifier.map(str::trim) {
        Some(BUNDLE_ID) => Ok(()),
        None | Some("") => Err(Refusal::UnknownIdentifier),
        Some(other) => Err(Refusal::OtherApp {
            identifier: other.to_owned(),
        }),
    }
}

/// The `CFBundleIdentifier` of the plist at `info_plist` (XML or binary), read with `plutil`;
/// `None` when plutil can't read it.
///
/// # Errors
///
/// Fails only when plutil can't be started.
pub(crate) fn read_identifier(info_plist: &Path) -> anyhow::Result<Option<String>> {
    let read = cmd::capture(
        Command::new(tool::PLUTIL)
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(info_plist),
    )?;
    Ok(identifier_from_plutil(read.status.success(), &read.stdout))
}

/// The identifier in plutil's answer: its trimmed stdout when it succeeded and printed one.
fn identifier_from_plutil(succeeded: bool, stdout: &str) -> Option<String> {
    let identifier = stdout.trim();
    (succeeded && !identifier.is_empty()).then(|| identifier.to_owned())
}

/// What is at a path a bundle is about to be written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Occupant {
    /// Nothing.
    Free,
    /// A bundle directory, with the path of its `Info.plist`.
    Bundle(PathBuf),
}

/// Looks at `path` without following symlinks.
///
/// # Errors
///
/// Fails when it can't be read, or holds a file, a symlink or a directory without
/// `Contents/Info.plist`: none of those can be told to be ours.
pub(crate) fn occupant(path: &Path) -> anyhow::Result<Occupant> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Occupant::Free),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    if !metadata.is_dir() {
        return Err(refusal(
            path,
            &"it is a file or a symlink, not a bundle directory",
        ));
    }
    let info = path.join("Contents/Info.plist");
    if !info.is_file() {
        return Err(refusal(path, &"it has no Contents/Info.plist"));
    }
    Ok(Occupant::Bundle(info))
}

/// Whether `path` holds this app (`true`) or nothing (`false`), reading bundle ids with
/// `identify`.
///
/// # Errors
///
/// Fails when [`occupant`] does, `identify` fails, or the bundle there is not this app.
pub(crate) fn holds_ours(
    path: &Path,
    identify: impl Fn(&Path) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<bool> {
    match occupant(path)? {
        Occupant::Free => Ok(false),
        Occupant::Bundle(info) => {
            may_replace(identify(&info)?.as_deref()).map_err(|why| refusal(path, &why))?;
            Ok(true)
        }
    }
}

/// The error for leaving `path` alone because of `why`.
fn refusal(path: &Path, why: &dyn fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(
        "refusing to replace {}: {why}; move it away yourself",
        path.display()
    )
}

#[cfg(test)]
mod tests;
