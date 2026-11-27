//! Reading an extension's zip safely: every entry a plain file or folder inside the extension's
//! folder (no absolute paths, no `..`, no links), within the size limits, and its manifest
//! naming Emberhut's extension.

use std::fs;
use std::io::{self, Cursor, Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::ExtensionError;
use super::id::extension_id;

/// The most entries a zip may hold.
const MAX_ENTRIES: usize = 5_000;
/// The most its files may hold together, unpacked.
const MAX_UNPACKED: u64 = 100 * 1024 * 1024;
/// A folder's mode.
const DIR_MODE: u32 = 0o755;
/// A file's mode.
const FILE_MODE: u32 = 0o644;

/// What a zip's manifest says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Its `version`.
    pub version: String,
}

/// One entry's checked place, relative to the extension's folder (the zip's one top folder
/// taken off).
fn checked_name(name: &str) -> Result<Vec<String>, ExtensionError> {
    let outside =
        || ExtensionError::Refused(format!("the zip's entry {name:?} is outside the folder"));
    if name.starts_with('/') || name.contains('\\') || name.contains('\0') || name.contains(':') {
        return Err(outside());
    }
    let parts: Vec<String> = name
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.iter().any(|part| part == ".." || part == ".") || parts.is_empty() {
        return Err(outside());
    }
    Ok(parts)
}

/// The manifest in `zip`, checked to be Emberhut's extension (`id`), and the folder prefix its
/// entries share (empty when the manifest is at the top).
///
/// # Errors
///
/// [`ExtensionError::Refused`] for a zip that isn't one, has no manifest, or whose manifest
/// names another extension or no version.
pub fn manifest(zip: &[u8], id: &str) -> Result<(Manifest, Vec<String>), ExtensionError> {
    let mut archive = open(zip)?;
    check_count(archive.len())?;
    let mut names = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| not_a_zip(&e))?;
        let name = entry.name().map_err(|e| not_a_zip(&e))?.into_owned();
        names.push(checked_name(&name)?);
    }
    let prefix: Vec<String> = if names.iter().any(|n| n == &["manifest.json"]) {
        Vec::new()
    } else {
        let top = names
            .first()
            .and_then(|n| n.first())
            .cloned()
            .unwrap_or_default();
        let one_folder = names.iter().all(|n| n.first() == Some(&top));
        let inside = names
            .iter()
            .any(|n| n.len() == 2 && n[0] == top && n[1] == "manifest.json");
        if !(one_folder && inside) {
            return Err(ExtensionError::Refused(
                "the zip has no manifest.json at its top or in its one folder".to_owned(),
            ));
        }
        vec![top]
    };
    let mut path = prefix.clone();
    path.push("manifest.json".to_owned());
    let mut text = Vec::new();
    archive
        .by_name(&path.join("/"))
        .map_err(|e| not_a_zip(&e))?
        .take(1024 * 1024)
        .read_to_end(&mut text)
        .map_err(|error| {
            ExtensionError::Refused(format!("its manifest.json can't be read: {error}"))
        })?;
    let manifest: Value = serde_json::from_slice(&text)
        .map_err(|_| ExtensionError::Refused("its manifest.json is not JSON".to_owned()))?;
    let key = manifest
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match extension_id(key) {
        Some(given) if given == id => {}
        Some(given) => {
            return Err(ExtensionError::Refused(format!(
                "its manifest's key is extension {given}'s, not Emberhut's ({id})"
            )));
        }
        None => {
            return Err(ExtensionError::Refused(
                "its manifest has no key, so its id would not be Emberhut's".to_owned(),
            ));
        }
    }
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ExtensionError::Refused("its manifest has no version".to_owned()))?;
    Ok((
        Manifest {
            version: version.to_owned(),
        },
        prefix,
    ))
}

/// Writes `zip`'s entries (less `prefix`) into the new, empty folder `into`.
///
/// # Errors
///
/// [`ExtensionError::Refused`] for a link, an encrypted entry, too many entries or too many
/// bytes; [`ExtensionError::Io`] when a file can't be written.
pub fn unpack(zip: &[u8], prefix: &[String], into: &Path) -> Result<(), ExtensionError> {
    let mut archive = open(zip)?;
    check_count(archive.len())?;
    let mut left = MAX_UNPACKED;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| not_a_zip(&e))?;
        let name = entry.name().map_err(|e| not_a_zip(&e))?.into_owned();
        let parts = checked_name(&name)?;
        if entry.enclosed_name().is_none() {
            return Err(ExtensionError::Refused(format!(
                "the zip's entry {name:?} is outside the folder"
            )));
        }
        if entry.is_symlink() {
            return Err(ExtensionError::Refused(format!(
                "the zip's entry {name:?} is a link"
            )));
        }
        if entry.encrypted() {
            return Err(ExtensionError::Refused(format!(
                "the zip's entry {name:?} is encrypted"
            )));
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170_000;
            if kind != 0 && kind != 0o100_000 && kind != 0o040_000 {
                return Err(ExtensionError::Refused(format!(
                    "the zip's entry {name:?} is not a file or a folder"
                )));
            }
        }
        let Some(inner) = parts.strip_prefix(prefix) else {
            return Err(ExtensionError::Refused(format!(
                "the zip's entry {name:?} is outside its folder"
            )));
        };
        let path: PathBuf = inner
            .iter()
            .fold(into.to_path_buf(), |path, part| path.join(part));
        if entry.is_dir() {
            make_dir(&path)?;
            continue;
        }
        if entry.size() > left {
            return Err(too_big());
        }
        if let Some(parent) = path.parent() {
            make_dir(parent)?;
        }
        let mut out = new_file(&path)?;
        let mut limited = (&mut entry).take(left + 1);
        let written = io::copy(&mut limited, &mut out).map_err(|error| {
            ExtensionError::Refused(format!("the zip's entry {name:?} can't be read: {error}"))
        })?;
        if written > left {
            return Err(too_big());
        }
        left -= written;
        out.flush()?;
    }
    Ok(())
}

/// Refuses a zip of more than [`MAX_ENTRIES`] entries.
fn check_count(entries: usize) -> Result<(), ExtensionError> {
    if entries > MAX_ENTRIES {
        return Err(ExtensionError::Refused(format!(
            "the zip holds more than {MAX_ENTRIES} entries"
        )));
    }
    Ok(())
}

/// The zip, opened.
fn open(zip: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, ExtensionError> {
    zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| not_a_zip(&e))
}

/// A zip the reader can't read.
fn not_a_zip(error: &zip::result::ZipError) -> ExtensionError {
    ExtensionError::Refused(format!("not a zip the helper can read ({error})"))
}

/// Unpacked, the zip is too big.
fn too_big() -> ExtensionError {
    ExtensionError::Refused(format!(
        "the zip unpacks to more than {} MB",
        MAX_UNPACKED / 1024 / 1024
    ))
}

/// Makes a folder (and its parents) inside the new folder.
fn make_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(path)
}

/// A new file; an entry named twice is refused rather than overwritten.
fn new_file(path: &Path) -> Result<fs::File, ExtensionError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                ExtensionError::Refused(format!("the zip names {} twice", path.display()))
            } else {
                ExtensionError::Io(error)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_inside_the_folder_pass_and_others_are_refused() {
        assert_eq!(checked_name("a/b.js").unwrap(), ["a", "b.js"]);
        assert_eq!(checked_name("dir/").unwrap(), ["dir"]);
        for bad in [
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a/./b",
            "a\\b",
            "c:x",
            "",
            "a\0b",
        ] {
            assert!(checked_name(bad).is_err(), "{bad:?}");
        }
    }
}
