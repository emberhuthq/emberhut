//! The JSON files the app keeps a value in (`theme.json`, `layout.json`, each room's kept chat):
//! read off the UI thread (most before the run loop starts) and written whole in the
//! background, newest first.
//!
//! A read gives nothing for a file that is missing (silently), unreadable or not the value
//! (each logged). A write goes through a temporary file and a rename, so a reader never sees
//! half a file, and a failed one leaves no temporary file behind. The files and the folders a
//! write makes are the user's only ([`private_files`]). Writes are numbered
//! ([`Writes`]): one that comes after a newer one is skipped, so the newest value is the one
//! on disk.

use std::io::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::private_files;

/// The value in the file at `path`, if it reads as one. `what` names the value in the log
/// ("a theme"), and `instead` says what the app does without it ("using the built-in one").
#[must_use]
pub(crate) fn read<T: DeserializeOwned>(path: &Path, what: &str, instead: &str) -> Option<T> {
    let bytes = std::fs::read(path)
        .inspect_err(|error| {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("can't read {}: {error}", path.display());
            }
        })
        .ok()?;
    serde_json::from_slice(&bytes)
        .inspect_err(|error| {
            log::warn!("{} isn't {what} ({error}); {instead}", path.display());
        })
        .ok()
}

/// Writes `value` to `path` as write number `number`, unless a newer one was written already
/// (`written` holds the number of the last one written). `what` names the value in the log
/// ("the theme").
pub(crate) fn write<T: Serialize>(
    path: &Path,
    value: &T,
    number: u64,
    written: &Mutex<u64>,
    what: &str,
) {
    let mut last = written.lock().unwrap_or_else(PoisonError::into_inner);
    if *last >= number {
        return;
    }
    let temporary = path.with_extension("json.tmp");
    let outcome = path
        .parent()
        .map_or(Ok(()), private_files::folder)
        .and_then(|()| serde_json::to_vec(value).map_err(std::io::Error::from))
        .and_then(|bytes| private_files::file(&temporary)?.write_all(&bytes))
        .and_then(|()| std::fs::rename(&temporary, path));
    match outcome {
        Ok(()) => *last = number,
        Err(error) => {
            log::warn!("can't keep {what} in {}: {error}", path.display());
            remove_temporary(&temporary);
        }
    }
}

/// Removes the temporary file a failed write may have left.
fn remove_temporary(temporary: &Path) {
    if let Err(error) = std::fs::remove_file(temporary)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("can't remove {}: {error}", temporary.display());
    }
}

/// The numbers of the writes to one file: each write takes the next one, and the file keeps
/// the number of the last one written.
#[derive(Debug, Default)]
pub(crate) struct Writes {
    /// The number of the last write handed out.
    sent: u64,
    /// The number of the last write that reached the file.
    written: Arc<Mutex<u64>>,
}

impl Writes {
    /// The next write's number, and what [`write()`] needs to tell whether a newer one came
    /// first.
    pub(crate) fn next(&mut self) -> (u64, Arc<Mutex<u64>>) {
        self.sent += 1;
        (self.sent, Arc::clone(&self.written))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct Value {
        n: u32,
    }

    #[test]
    fn a_value_written_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new").join("value.json");
        assert_eq!(read::<Value>(&path, "a value", "none"), None, "missing");
        write(&path, &Value { n: 3 }, 1, &Mutex::new(0), "the value");
        assert_eq!(read(&path, "a value", "none"), Some(Value { n: 3 }));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_the_folder_it_makes_are_the_users_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new").join("value.json");
        write(&path, &Value { n: 3 }, 1, &Mutex::new(0), "the value");
        assert_eq!(private_files::mode(&path), 0o600);
        assert_eq!(private_files::mode(path.parent().unwrap()), 0o700);
    }

    #[test]
    fn a_file_that_isnt_the_value_reads_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("value.json");
        for text in ["", "{", r#"{"n": "three"}"#, "[]"] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(read::<Value>(&path, "a value", "none"), None, "{text}");
        }
    }

    #[test]
    fn a_write_that_comes_after_a_newer_one_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("value.json");
        let mut writes = Writes::default();
        let (older, written) = writes.next();
        let (newer, _) = writes.next();
        assert!(older < newer);
        write(&path, &Value { n: 2 }, newer, &written, "the value");
        write(&path, &Value { n: 1 }, older, &written, "the value");
        assert_eq!(read(&path, "a value", "none"), Some(Value { n: 2 }));
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file goes: the rename fails.
        let path = dir.path().join("value.json");
        std::fs::create_dir(&path).unwrap();
        let written = Mutex::new(0);
        write(&path, &Value { n: 1 }, 1, &written, "the value");
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(*written.lock().unwrap(), 0, "not written");
    }
}
