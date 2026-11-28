//! A session kept in a file.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::session::{Session, SessionStore, StoreError, stored_version};

/// A session kept as JSON in a file only its user can read and write (0600 on Unix, in a
/// folder made 0700 when this store creates it).
///
/// A save writes a new file next to it and renames it over the old one, so the file is always
/// a whole session: a crash mid-save leaves the one before.
///
/// One process per file: renewals are one at a time within a [`crate::Client`], not across
/// processes, so two processes renewing over one file can sign each other out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSessionStore {
    path: PathBuf,
}

/// Tells this process's temporary files apart.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

impl FileSessionStore {
    /// A store at this path (its folder is made on the first save).
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Its file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn io_error(&self, source: io::Error) -> StoreError {
        StoreError::Io {
            path: self.path.clone(),
            source,
        }
    }

    /// [`StoreError::NewerFormat`] when the file holds a newer client's session: a save never
    /// replaces one, whichever path it comes from. (A newer client writing between this check
    /// and the rename isn't caught: one process per file.)
    fn refuse_newer(&self) -> Result<(), StoreError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(self.io_error(error)),
        };
        match stored_version(&bytes) {
            Some(version) if version > Session::FORMAT_VERSION => Err(StoreError::NewerFormat {
                path: self.path.clone(),
                version,
            }),
            _ => Ok(()),
        }
    }

    fn folder(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    /// Writes `bytes` to a new private file in the store's folder, flushed to disk.
    fn write_temporary(&self, bytes: &[u8]) -> io::Result<PathBuf> {
        let name = self
            .path
            .file_name()
            .map_or_else(|| "session".into(), |n| n.to_string_lossy());
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .folder()
            .join(format!(".{name}.{}.{unique}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let written = options.open(&temporary).and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
        match written {
            Ok(()) => Ok(temporary),
            Err(error) => {
                remove_temporary(&temporary);
                Err(error)
            }
        }
    }
}

/// Removes a temporary file that won't be used; a failure only leaves a stray file behind.
fn remove_temporary(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), %error, "couldn't remove a temporary session file");
    }
}

fn create_private_folder(folder: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder)
}

impl SessionStore for FileSessionStore {
    fn load(&self) -> Result<Option<Session>, StoreError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(self.io_error(error)),
        };
        if let Some(version) = stored_version(&bytes).filter(|v| *v > Session::FORMAT_VERSION) {
            return Err(StoreError::NewerFormat {
                path: self.path.clone(),
                version,
            });
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| StoreError::Corrupt {
                path: self.path.clone(),
                source,
            })
    }

    fn save(&self, session: &Session) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec_pretty(session).map_err(StoreError::Encode)?;
        create_private_folder(self.folder()).map_err(|e| self.io_error(e))?;
        let temporary = self.write_temporary(&bytes).map_err(|e| self.io_error(e))?;
        if let Err(error) = self.refuse_newer() {
            remove_temporary(&temporary);
            return Err(error);
        }
        if let Err(error) = fs::rename(&temporary, &self.path) {
            remove_temporary(&temporary);
            return Err(self.io_error(error));
        }
        // The rename is durable once the folder's entry is on disk too.
        #[cfg(unix)]
        fs::File::open(self.folder())
            .and_then(|folder| folder.sync_all())
            .map_err(|e| self.io_error(e))?;
        Ok(())
    }

    fn clear(&self) -> Result<(), StoreError> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(self.io_error(error)),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system's temporary folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "emberhut-client-{test}-{}-{unique}",
                std::process::id()
            ));
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
            }
        }
    }

    #[test]
    fn nothing_saved_reads_as_none() {
        let scratch = Scratch::new("none");
        let store = FileSessionStore::new(scratch.0.join("session.json"));
        assert!(store.load().unwrap().is_none());
        store.clear().unwrap();
    }

    #[test]
    fn saves_load_back_and_clear() {
        let scratch = Scratch::new("save");
        let store = FileSessionStore::new(scratch.0.join("nested/session.json"));
        let session = Session::new_install().unwrap();
        store.save(&session).unwrap();
        assert_eq!(store.load().unwrap(), Some(session.clone()));
        let mut second = session;
        second.install = emberhut_api::ids::InstallId::new("zr-second");
        store.save(&second).unwrap();
        assert_eq!(store.load().unwrap(), Some(second));
        let leftovers: Vec<_> = fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .collect();
        assert_eq!(leftovers.len(), 1, "no temporary files are left behind");
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn only_its_user_can_read_it() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("mode");
        let store = FileSessionStore::new(scratch.0.join("private/session.json"));
        store.save(&Session::new_install().unwrap()).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.path()), 0o600);
        assert_eq!(mode(store.path().parent().unwrap()), 0o700);
    }

    #[test]
    fn a_file_that_isnt_a_session_is_corrupt() {
        let scratch = Scratch::new("corrupt");
        fs::create_dir_all(&scratch.0).unwrap();
        let path = scratch.0.join("session.json");
        fs::write(&path, b"not json").unwrap();
        let error = FileSessionStore::new(&path).load().unwrap_err();
        assert!(matches!(error, StoreError::Corrupt { .. }), "{error}");
        assert!(error.to_string().contains("session.json"));
    }

    #[test]
    fn a_newer_clients_session_is_neither_read_nor_overwritten() {
        let scratch = Scratch::new("newer");
        fs::create_dir_all(&scratch.0).unwrap();
        let path = scratch.0.join("session.json");
        fs::write(
            &path,
            br#"{"version": 9, "install": "zr-newer", "somethingNew": true}"#,
        )
        .unwrap();
        let error = FileSessionStore::new(&path).load().unwrap_err();
        assert!(
            matches!(error, StoreError::NewerFormat { version: 9, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_save_never_replaces_a_newer_clients_session() {
        let scratch = Scratch::new("newer-save");
        fs::create_dir_all(&scratch.0).unwrap();
        let path = scratch.0.join("session.json");
        let newer = br#"{"version": 9, "install": "zr-newer"}"#;
        fs::write(&path, newer).unwrap();
        let error = FileSessionStore::new(&path)
            .save(&Session::new_install().unwrap())
            .unwrap_err();
        assert!(
            matches!(error, StoreError::NewerFormat { version: 9, .. }),
            "{error}"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            newer,
            "the newer file is as it was"
        );
        assert_eq!(
            fs::read_dir(&scratch.0).unwrap().count(),
            1,
            "no temporary file left"
        );
    }

    #[test]
    fn a_save_replaces_a_session_of_this_format() {
        let scratch = Scratch::new("same-save");
        let store = FileSessionStore::new(scratch.0.join("session.json"));
        store.save(&Session::new_install().unwrap()).unwrap();
        let second = Session::new_install().unwrap();
        store.save(&second).unwrap();
        assert_eq!(store.load().unwrap(), Some(second));
    }

    #[test]
    fn an_unwritable_place_is_an_io_error() {
        let scratch = Scratch::new("unwritable");
        fs::create_dir_all(&scratch.0).unwrap();
        let blocker = scratch.0.join("file");
        fs::write(&blocker, b"").unwrap();
        let store = FileSessionStore::new(blocker.join("session.json"));
        let error = store.save(&Session::new_install().unwrap()).unwrap_err();
        assert!(matches!(error, StoreError::Io { .. }), "{error}");
    }
}
