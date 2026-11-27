//! Filesystem helpers shared by the tasks.

use std::fs;
use std::io;
use std::path::Path;

use anyhow::Context as _;

/// Removes `dir` and everything in it, if it exists.
///
/// # Errors
///
/// Fails when it exists but can't be removed.
pub(crate) fn remove_dir_if_present(dir: &Path) -> anyhow::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            Err(err).with_context(|| format!("removing {}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// Removes the file at `path`, if it exists.
///
/// # Errors
///
/// Fails when it exists but can't be removed.
pub(crate) fn remove_file_if_present(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            Err(err).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}
