//! The browser extension's folder, which Chrome loads unpacked and the helper keeps up to date:
//! `GET /extension` says which version is there, `POST /extension/install` puts a new zip in
//! place. A zip is checked whole (Emberhut's extension id, every entry inside the folder, no links,
//! the size limits), unpacked into a new folder beside the old one, and swapped into place in
//! one rename, so Chrome never sees half an extension.

pub mod id;
pub mod unpack;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The largest zip taken: 20 MB (20,000,000 bytes, as the contract says).
pub const MAX_ZIP: usize = 20_000_000;

/// Why a zip was not installed.
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    /// The zip is refused; the words say why.
    #[error("{0}")]
    Refused(String),
    /// The folder could not be written.
    #[error("the extension's folder could not be written")]
    Io(#[from] io::Error),
}

/// The extension installed: its version and folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// Its manifest's version.
    pub version: String,
    /// Its folder.
    pub path: PathBuf,
}

/// The extension's folder.
#[derive(Clone, Debug)]
pub struct ExtensionFolder {
    dir: PathBuf,
    id: String,
}

impl ExtensionFolder {
    /// The folder `dir`, taking zips of the extension `id` only.
    #[must_use]
    pub fn new(dir: PathBuf, id: &str) -> Self {
        Self {
            dir,
            id: id.to_owned(),
        }
    }

    /// The extension in the folder, when there is one with a version.
    #[must_use]
    pub fn installed(&self) -> Option<Installed> {
        let text = fs::read(self.dir.join("manifest.json")).ok()?;
        let manifest: Value = serde_json::from_slice(&text).ok()?;
        let version = manifest.get("version")?.as_str()?.to_owned();
        Some(Installed {
            version,
            path: self.dir.clone(),
        })
    }

    /// Checks `zip`, unpacks it beside the folder and swaps it into place.
    ///
    /// # Errors
    ///
    /// [`ExtensionError::Refused`] for a zip that isn't Emberhut's extension or isn't safe to
    /// unpack; [`ExtensionError::Io`] when the folder can't be written. Either way the folder
    /// is left as it was.
    pub fn install(&self, zip: &[u8]) -> Result<Installed, ExtensionError> {
        if zip.len() > MAX_ZIP {
            return Err(ExtensionError::Refused("the zip is over 20 MB".to_owned()));
        }
        let (manifest, prefix) = unpack::manifest(zip, &self.id)?;
        let parent = self.dir.parent().ok_or_else(|| {
            ExtensionError::Refused("the extension's folder has no parent".to_owned())
        })?;
        fs::create_dir_all(parent)?;
        let name = self
            .dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Extension");
        clear_leftovers(parent, name);
        let fresh = parent.join(format!(".{name}.new-{}", suffix()?));
        fs::create_dir(&fresh)?;
        let placed = unpack::unpack(zip, &prefix, &fresh).and_then(|()| swap(&fresh, &self.dir));
        if let Err(error) = placed {
            if let Err(left) = fs::remove_dir_all(&fresh) {
                tracing::warn!(error = %left, "an unpacked extension was left beside the folder");
            }
            return Err(error);
        }
        tracing::info!(version = %manifest.version, "installed the browser extension");
        Ok(Installed {
            version: manifest.version,
            path: self.dir.clone(),
        })
    }
}

/// A random suffix for the new folder's name.
fn suffix() -> Result<String, ExtensionError> {
    let mut bytes = [0_u8; 6];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| ExtensionError::Io(io::Error::other(error)))?;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        crate::text::push(&mut text, format_args!("{byte:02x}"));
    }
    Ok(text)
}

/// Removes new folders an interrupted install left beside the folder `name` in `parent`.
fn clear_leftovers(parent: &Path, name: &str) {
    let start = format!(".{name}.new-");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.file_name().to_string_lossy().starts_with(&start)
            && let Err(error) = fs::remove_dir_all(entry.path())
        {
            tracing::warn!(%error, "a left-over extension folder was not removed");
        }
    }
}

/// Puts `fresh` in place at `dir` in one rename: exchanged with the old folder when there is
/// one (which is then removed), else moved there.
fn swap(fresh: &Path, dir: &Path) -> Result<(), ExtensionError> {
    if dir.exists() {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            fresh,
            rustix::fs::CWD,
            dir,
            rustix::fs::RenameFlags::EXCHANGE,
        )
        .map_err(io::Error::from)?;
        // `fresh` now holds the old extension.
        if let Err(error) = fs::remove_dir_all(fresh) {
            tracing::warn!(%error, "the old extension was not removed");
        }
    } else {
        fs::rename(fresh, dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
