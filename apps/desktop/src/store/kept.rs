//! The app's folder of files fetched to open: where each is kept (a name safe on every disk,
//! in a folder of its own server address), fetching one into it (streamed into a private
//! partial, [`PARTIAL`], moved into place when whole, never left half), and clearing out the
//! old ones. The folder is private: 0700, its files 0600, on Unix.

use std::fs::File;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::backend::{Backend, Error, TooLarge};
use crate::private_files;

/// How long a kept file stays once nothing opens it.
const KEEP_FOR: Duration = Duration::from_hours(24);

/// How old a partial is before it counts as left over (no fetch takes that long without a
/// write, and one under way may be minutes old).
const PARTIAL_FOR: Duration = Duration::from_hours(1);

/// The name of the file a fetch writes into, in the folder of the file's address: hidden, and
/// one no kept file can have (see `file_name`).
const PARTIAL: &str = ".emberhut-part";

/// The longest folder name made from an address before the rest of it is hashed.
const FOLDER_MAX: usize = 100;

/// The longest file name kept, in bytes (file systems take 255).
const NAME_MAX: usize = 150;

/// Where a chat file is kept once fetched: a folder named after its whole address (the
/// server's with the file's path, so two servers' files never share one), every byte but ASCII
/// letters and digits escaped (`_` and its hex) so two addresses never share one, then its own
/// name, made safe as one path component.
#[must_use]
pub(crate) fn kept_path(root: &Path, url: &str, name: &str) -> PathBuf {
    root.join(folder_for(url)).join(file_name(name))
}

/// The folder for an address, one of its own for each (see [`kept_path`]); a long one keeps
/// its start and a hash of the whole.
pub(crate) fn folder_for(url: &str) -> String {
    let mut folder = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii_alphanumeric() {
            folder.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            folder.push('_');
            folder.push(char::from(HEX[usize::from(byte >> 4)]));
            folder.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    if folder.len() <= FOLDER_MAX {
        return folder;
    }
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let start: String = folder.chars().take(FOLDER_MAX - 17).collect();
    format!("{start}-{:016x}", hasher.finish())
}

/// A name safe as one path component on every platform, at most [`NAME_MAX`] bytes, its
/// extension kept.
fn file_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let name = match name.trim() {
        "" | "." | ".." => return "file".to_owned(),
        PARTIAL => return PARTIAL.trim_start_matches('.').to_owned(),
        trimmed => trimmed.to_owned(),
    };
    if name.len() <= NAME_MAX {
        return name;
    }
    let extension = Path::new(&name)
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .filter(|ext| ext.len() <= 20)
        .map_or_else(String::new, |ext| format!(".{ext}"));
    let mut stem = String::new();
    for c in name.chars() {
        if stem.len() + c.len_utf8() + extension.len() > NAME_MAX {
            break;
        }
        stem.push(c);
    }
    stem + &extension
}

/// Why a file couldn't be had.
fn fetch_failure(error: &Error) -> String {
    match error {
        Error::Local(cause) if cause.is::<TooLarge>() => "Too large to open here".to_owned(),
        Error::Local(_) => "Couldn't save it on this device".to_owned(),
        _ => "Couldn't fetch it from the server".to_owned(),
    }
}

/// The file at `path` (the server's `url`): kept already (it counts as used now), or streamed
/// into a private [`PARTIAL`] beside it, then moved into place; a failed one leaves nothing.
pub(crate) fn fetch(
    backend: &dyn Backend,
    url: &str,
    path: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
) -> Result<(), String> {
    if path.is_file() {
        if let Err(error) = File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(SystemTime::now()))
        {
            log::debug!("couldn't mark {} as used: {error}", path.display());
        }
        return Ok(());
    }
    let local = |error: io::Error| {
        log::warn!("couldn't keep {}: {error}", path.display());
        "Couldn't save it on this device".to_owned()
    };
    if let Some(folder) = path.parent() {
        private_files::folder(folder).map_err(local)?;
    }
    let partial = path.with_file_name(PARTIAL);
    let mut file = private_files::file(&partial).map_err(local)?;
    let fetched = backend
        .download(url, &mut file, progress)
        .map_err(|error| {
            log::warn!("couldn't fetch {url}: {error}");
            fetch_failure(&error)
        })
        .and_then(|_| file.sync_all().map_err(local))
        .and_then(|()| std::fs::rename(&partial, path).map_err(local));
    if fetched.is_err()
        && let Err(error) = std::fs::remove_file(&partial)
        && error.kind() != io::ErrorKind::NotFound
    {
        log::warn!("couldn't remove {}: {error}", partial.display());
    }
    fetched
}

/// Removes, as of `now`, the kept files under `root` untouched for a day ([`KEEP_FOR`]), the
/// partials left over for an hour ([`PARTIAL_FOR`]: a younger one may be a fetch under way),
/// and the folders that leaves empty.
pub(crate) fn sweep(root: &Path, now: SystemTime) {
    let before = |age: Duration| now.checked_sub(age).unwrap_or(SystemTime::UNIX_EPOCH);
    let (kept_before, partial_before) = (before(KEEP_FOR), before(PARTIAL_FOR));
    let Ok(folders) = std::fs::read_dir(root) else {
        return;
    };
    for folder in folders.flatten().map(|entry| entry.path()) {
        let Ok(files) = std::fs::read_dir(&folder) else {
            continue;
        };
        for file in files.flatten() {
            let limit = if file.file_name() == PARTIAL {
                partial_before
            } else {
                kept_before
            };
            let old = file
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified < limit);
            if old && let Err(error) = std::fs::remove_file(file.path()) {
                log::debug!("couldn't clear {}: {error}", file.path().display());
            }
        }
        // Only an empty folder goes; a folder still in use fails here, which is fine.
        if std::fs::remove_dir(&folder).is_ok() {
            log::debug!("cleared {}", folder.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_kept_under_its_own_name_in_a_folder_of_its_address() {
        let root = Path::new("/cache/files");
        assert_eq!(
            kept_path(root, "/uploads/1a2b-report.pdf", "report.pdf"),
            Path::new("/cache/files/_2fuploads_2f1a2b_2dreport_2epdf/report.pdf")
        );
    }

    #[test]
    fn two_addresses_never_share_a_folder() {
        assert_ne!(
            folder_for("/uploads/x-a-b.pdf"),
            folder_for("/uploads/x-a.b.pdf")
        );
        assert_ne!(folder_for("/uploads/a_2d"), folder_for("/uploads/a-"));
        let long = format!("/uploads/{}", "é".repeat(100));
        let folder = folder_for(&long);
        assert!(folder.len() <= FOLDER_MAX, "{folder}");
        assert_ne!(folder, folder_for(&format!("{long}x")));
    }

    #[test]
    fn a_name_never_leaves_its_folder_and_fits_any_disk() {
        for (name, kept) in [
            ("../../etc/passwd", "..-..-etc-passwd"),
            ("..", "file"),
            (" ", "file"),
            ("a\\b:c", "a-b-c"),
            ("what? <now> \"x\" | y*.txt", "what- -now- -x- - y-.txt"),
        ] {
            assert_eq!(file_name(name), kept, "{name}");
        }
        let long = format!("{}.pdf", "تقرير".repeat(40));
        let kept = file_name(&long);
        assert!(kept.len() <= NAME_MAX, "{kept}");
        assert_eq!(Path::new(&kept).extension(), Some("pdf".as_ref()), "{kept}");
    }

    /// `path`, written now and last touched `age` ago.
    fn written(path: &Path, age: Duration) {
        private_files::folder(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn the_sweep_clears_old_files_and_old_partials_only() {
        let root = tempfile::tempdir().unwrap();
        let at = |folder: &str, name: &str| root.path().join(folder).join(name);
        let minute = Duration::from_secs(60);
        let (old, fresh, named_part) = (
            at("a", "old.pdf"),
            at("b", "fresh.pdf"),
            at("c", "notes.part"),
        );
        let (left_over, under_way) = (at("d", PARTIAL), at("e", PARTIAL));
        written(&old, KEEP_FOR + minute);
        written(&fresh, minute);
        written(&named_part, minute);
        written(&left_over, PARTIAL_FOR + minute);
        written(&under_way, minute);

        sweep(root.path(), SystemTime::now());

        assert!(!old.exists() && !old.parent().unwrap().exists());
        assert!(!left_over.exists());
        assert!(fresh.exists());
        assert!(
            named_part.exists(),
            "a kept file named .part is a kept file"
        );
        assert!(under_way.exists(), "a fetch under way keeps its partial");
    }

    #[test]
    fn no_kept_file_is_named_as_a_partial() {
        assert_ne!(file_name(PARTIAL), PARTIAL);
        let path = kept_path(Path::new("/cache"), "/uploads/n.part", "n.part");
        assert_ne!(path.with_file_name(PARTIAL), path);
        assert_eq!(path.file_name(), Some("n.part".as_ref()));
    }

    #[test]
    fn the_failure_says_what_went_wrong() {
        let too_large = Error::Local(Box::new(TooLarge { size: 9, limit: 4 }));
        assert_eq!(fetch_failure(&too_large), "Too large to open here");
        let disk = Error::Local("disk full".into());
        assert_eq!(fetch_failure(&disk), "Couldn't save it on this device");
        let gone = Error::Refused {
            status: Some(404),
            message: None,
        };
        assert_eq!(fetch_failure(&gone), "Couldn't fetch it from the server");
    }
}
