//! A step's files, as the server reads them: from the path the agent gave, never trusting its
//! word for their bytes. A request's files are shown (and uploaded) to its approver, so what may
//! leave is checked on the file actually read, not on a path:
//!
//! - the file is opened once; the private and hidden checks run on the real path of the open
//!   handle (on Linux, `/proc/self/fd`; on macOS and Windows, the canonical path, which must
//!   still be the opened file), so a link swapped in between the check and the read changes
//!   nothing;
//! - a private file (the secrets folder, the server's keys, the owner's key folders), a hidden
//!   one, a system file (under `/proc`, `/sys` or `/dev`, whose size says nothing of what it
//!   holds), an empty one and one with other names (a hard link to a private file has an
//!   innocent path) are refused;
//! - at most [`FILE_MAX`] bytes are read, whatever the size said;
//! - the open handle is kept, and what is sent to the approver is read from it again and must
//!   still have the hash; the path is never opened twice.

use std::fs::File;
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use emberhut_core::approvals::{FILES_MAX, FileDigest};

use crate::services::private::PrivatePaths;

/// The biggest file a request carries.
pub const FILE_MAX: u64 = 100 << 20;

/// The folders of the system's own files, never sent.
const SYSTEM: [&str; 3] = ["/proc", "/sys", "/dev"];

/// A file of a step, read now, with its open handle.
#[derive(Clone, Debug)]
pub(crate) struct StepFile {
    /// Its name, size and hash.
    pub(crate) digest: FileDigest,
    /// The file as it was opened and read, to read again when it is sent to the approver.
    handle: Arc<File>,
}

/// Why a step's file is refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FileRefused {
    /// More files than a request carries.
    #[error("a request carries at most {FILES_MAX} files")]
    TooMany,
    /// A path that isn't absolute.
    #[error("{0}: give the file's absolute path")]
    NotAbsolute(String),
    /// A path that can't be read.
    #[error("{path}: {source}")]
    Unreadable {
        /// The path as given.
        path: String,
        /// Why not.
        #[source]
        source: io::Error,
    },
    /// Not a regular file.
    #[error("{0}: not a file")]
    NotAFile(String),
    /// A file that never leaves the server.
    #[error("{0}: a private file never leaves the server")]
    Private(String),
    /// A hidden file, or one in a hidden folder.
    #[error("{0}: a hidden file is not sent")]
    Hidden(String),
    /// One of the system's own files.
    #[error("{0}: a system file is not sent")]
    System(String),
    /// An empty file.
    #[error("{0}: the file is empty")]
    Empty(String),
    /// A file with other names (hard links), whose path says nothing of what it is.
    #[error("{0}: a file with other names (hard links) is not sent")]
    Linked(String),
    /// A file that changed while it was being opened.
    #[error("{0}: the file changed while it was read; try again")]
    Changed(String),
    /// Over [`FILE_MAX`].
    #[error("{0}: a file is at most 100 MB")]
    TooLarge(String),
    /// Outside the folders a configured agent's files come from.
    #[error("{0}: not in your own folder or your tasks' folders")]
    Outside(String),
}

/// Reads each of `paths`, in order, refusing what may not be sent; with `within`, also any
/// whose real path (the open handle's) is under none of those folders (a configured agent's
/// roots).
pub(crate) async fn read_all(
    paths: &[PathBuf],
    private: &PrivatePaths,
    within: Option<&[PathBuf]>,
) -> Result<Vec<StepFile>, FileRefused> {
    let within: Option<Vec<PathBuf>> = within.map(|roots| {
        roots
            .iter()
            .filter_map(|root| std::fs::canonicalize(root).ok())
            .collect()
    });
    if paths.len() > FILES_MAX {
        return Err(FileRefused::TooMany);
    }
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path.clone();
        let private = private.clone();
        let within = within.clone();
        let read =
            tokio::task::spawn_blocking(move || read_one(&path, &private, within.as_deref()))
                .await
                .map_err(|error| FileRefused::Unreadable {
                    path: String::new(),
                    source: io::Error::other(error),
                })??;
        files.push(read);
    }
    Ok(files)
}

/// The bytes of `file`, read again from its open handle, if they still have its hash.
pub(crate) async fn bytes_if_unchanged(file: &StepFile) -> Option<Vec<u8>> {
    let handle = Arc::clone(&file.handle);
    let digest = file.digest.clone();
    tokio::task::spawn_blocking(move || {
        let mut reader = &*handle;
        reader.seek(SeekFrom::Start(0)).ok()?;
        let bytes = read_capped(reader).ok()??;
        (FileDigest::of(digest.name.clone(), &bytes) == digest).then_some(bytes)
    })
    .await
    .ok()
    .flatten()
}

/// Up to [`FILE_MAX`] bytes of `reader`; none when there are more.
fn read_capped(reader: &File) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader.take(FILE_MAX + 1).read_to_end(&mut bytes)?;
    Ok((u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= FILE_MAX).then_some(bytes))
}

fn read_one(
    path: &Path,
    private: &PrivatePaths,
    within: Option<&[PathBuf]>,
) -> Result<StepFile, FileRefused> {
    let shown = path.display().to_string();
    if !path.is_absolute() {
        return Err(FileRefused::NotAbsolute(shown));
    }
    let unreadable = |source| FileRefused::Unreadable {
        path: shown.clone(),
        source,
    };
    let handle = File::open(path).map_err(unreadable)?;
    let real = real_path(path, &handle).map_err(|error| match error {
        RealPath::Io(source) => unreadable(source),
        RealPath::Changed => FileRefused::Changed(shown.clone()),
    })?;
    if SYSTEM.iter().any(|root| real.starts_with(root)) {
        return Err(FileRefused::System(shown));
    }
    if private.holds(&real) {
        return Err(FileRefused::Private(shown));
    }
    // Within a configured agent's roots, only what is under the root counts as hidden: the
    // data folder the roots are in may itself be a hidden one.
    let hidden = match within {
        Some(roots) => match roots.iter().find_map(|root| real.strip_prefix(root).ok()) {
            Some(inside) => is_hidden(inside),
            None => return Err(FileRefused::Outside(shown)),
        },
        None => is_hidden(&real) || is_hidden(path),
    };
    if hidden {
        return Err(FileRefused::Hidden(shown));
    }
    let meta = handle.metadata().map_err(unreadable)?;
    if !meta.is_file() {
        return Err(FileRefused::NotAFile(shown));
    }
    if meta.len() == 0 {
        return Err(FileRefused::Empty(shown));
    }
    if has_other_names(&handle, &meta) {
        return Err(FileRefused::Linked(shown));
    }
    if meta.len() > FILE_MAX {
        return Err(FileRefused::TooLarge(shown));
    }
    let bytes = read_capped(&handle)
        .map_err(unreadable)?
        .ok_or_else(|| FileRefused::TooLarge(shown.clone()))?;
    if bytes.is_empty() {
        return Err(FileRefused::Empty(shown));
    }
    let name = path
        .file_name()
        .map_or_else(|| shown.clone(), |name| name.to_string_lossy().into_owned());
    Ok(StepFile {
        digest: FileDigest::of(name, &bytes),
        handle: Arc::new(handle),
    })
}

/// Why the real path of an open file is not known.
enum RealPath {
    /// The system didn't say.
    Io(io::Error),
    /// The path no longer names the opened file.
    #[cfg_attr(
        any(target_os = "linux", not(any(unix, windows))),
        expect(
            dead_code,
            reason = "on Linux the handle names its own path, so it can't have changed; \
                      elsewhere no path is known"
        )
    )]
    Changed,
}

/// The real path of `handle`, opened from `path`: the handle's own, as the kernel has it.
#[cfg(target_os = "linux")]
fn real_path(_: &Path, handle: &File) -> Result<PathBuf, RealPath> {
    use std::os::fd::AsRawFd as _;
    std::fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd())).map_err(RealPath::Io)
}

/// The real path of `handle`, opened from `path`: the canonical path, which must still be the
/// opened file (the same device and inode, and not a link now), so a swap is refused.
#[cfg(all(unix, not(target_os = "linux")))]
fn real_path(path: &Path, handle: &File) -> Result<PathBuf, RealPath> {
    use std::os::unix::fs::MetadataExt as _;
    let real = std::fs::canonicalize(path).map_err(RealPath::Io)?;
    let opened = handle.metadata().map_err(RealPath::Io)?;
    let named = std::fs::symlink_metadata(&real).map_err(RealPath::Io)?;
    if named.file_type().is_symlink() || (named.dev(), named.ino()) != (opened.dev(), opened.ino())
    {
        return Err(RealPath::Changed);
    }
    Ok(real)
}

/// The real path of `handle`, opened from `path`: the canonical path, which must still be the
/// opened file (the same volume and file index), so a swap is refused.
#[cfg(windows)]
fn real_path(path: &Path, handle: &File) -> Result<PathBuf, RealPath> {
    use winapi_util::file::{Information, information};
    let identity = |info: &Information| (info.volume_serial_number(), info.file_index());
    let real = std::fs::canonicalize(path).map_err(RealPath::Io)?;
    let opened = information(handle).map_err(RealPath::Io)?;
    let named = winapi_util::Handle::from_path_any(&real)
        .and_then(|named| information(&named))
        .map_err(RealPath::Io)?;
    if identity(&named) != identity(&opened) {
        return Err(RealPath::Changed);
    }
    Ok(real)
}

/// The real path of an open file, where the system can't tell it: unknown, so nothing is sent.
#[cfg(not(any(unix, windows)))]
fn real_path(_: &Path, _: &File) -> Result<PathBuf, RealPath> {
    Err(RealPath::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        "this system can't say which file was opened",
    )))
}

/// Whether the open file has other names: a hard link elsewhere.
#[cfg(unix)]
fn has_other_names(_: &File, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    meta.nlink() > 1
}

/// Whether the open file has other names: a hard link elsewhere (when the system can't say,
/// it is taken to have some, so it isn't sent).
#[cfg(windows)]
fn has_other_names(handle: &File, _: &std::fs::Metadata) -> bool {
    winapi_util::file::information(handle).map_or(true, |info| info.number_of_links() > 1)
}

/// Whether the open file has other names: not known on this system, so none.
#[cfg(not(any(unix, windows)))]
fn has_other_names(_: &File, _: &std::fs::Metadata) -> bool {
    false
}

/// Whether a part of `path` is hidden (starts with a dot).
fn is_hidden(path: &Path) -> bool {
    path.components().any(|part| match part {
        Component::Normal(name) => name.as_encoded_bytes().starts_with(b"."),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("approval-files")
            .tempdir()
            .unwrap()
    }

    #[tokio::test]
    async fn a_file_is_hashed_from_its_bytes_and_a_private_or_hidden_one_is_refused() {
        let dir = folder();
        let secrets = dir.path().join("secrets");
        std::fs::create_dir(&secrets).unwrap();
        std::fs::write(secrets.join("key"), "k").unwrap();
        let file = dir.path().join("cover.png");
        std::fs::write(&file, b"png bytes").unwrap();
        let hidden = dir.path().join(".env");
        std::fs::write(&hidden, b"x").unwrap();
        let private = PrivatePaths::new([secrets.clone()]);

        let read = read_all(std::slice::from_ref(&file), &private, None)
            .await
            .unwrap();
        assert_eq!(read[0].digest, FileDigest::of("cover.png", b"png bytes"));
        assert!(bytes_if_unchanged(&read[0]).await.is_some());

        // A link to a private file is refused by its real path (links need Unix here: on
        // Windows, making one takes a privilege).
        #[cfg(unix)]
        {
            let link = dir.path().join("innocent.txt");
            std::os::unix::fs::symlink(secrets.join("key"), &link).unwrap();
            assert!(matches!(
                read_all(&[link], &private, None).await,
                Err(FileRefused::Private(_))
            ));
        }
        assert!(matches!(
            read_all(&[hidden], &private, None).await,
            Err(FileRefused::Hidden(_))
        ));
        assert!(matches!(
            read_all(&[PathBuf::from("cover.png")], &private, None).await,
            Err(FileRefused::NotAbsolute(_))
        ));
        assert!(matches!(
            read_all(&[dir.path().to_path_buf()], &private, None).await,
            Err(FileRefused::NotAFile(_))
        ));
        assert!(matches!(
            read_all(&[dir.path().join("missing")], &private, None).await,
            Err(FileRefused::Unreadable { .. })
        ));
        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        assert!(matches!(
            read_all(&[empty], &private, None).await,
            Err(FileRefused::Empty(_))
        ));
    }

    /// The system's pseudo-files say they are empty and hold anything: the server's own
    /// environment among them.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_system_file_is_refused() {
        let private = PrivatePaths::default();
        for path in ["/proc/self/environ", "/proc/self/cmdline", "/dev/zero"] {
            assert!(
                matches!(
                    read_all(&[PathBuf::from(path)], &private, None).await,
                    Err(FileRefused::System(_))
                ),
                "{path}"
            );
        }
        let dir = folder();
        let link = dir.path().join("report.txt");
        std::os::unix::fs::symlink("/proc/self/environ", &link).unwrap();
        assert!(matches!(
            read_all(&[link], &private, None).await,
            Err(FileRefused::System(_))
        ));
    }

    /// What is sent is read from the handle that was checked: a link swapped in at the path
    /// afterwards sends nothing of what it points at.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_swapped_in_after_the_check_sends_the_file_that_was_checked() {
        let dir = folder();
        let secrets = dir.path().join("secrets");
        std::fs::create_dir(&secrets).unwrap();
        std::fs::write(secrets.join("id_key"), b"private key bytes").unwrap();
        let private = PrivatePaths::new([secrets.clone()]);
        let report = dir.path().join("report.pdf");
        std::fs::write(&report, b"report bytes").unwrap();

        let read = read_all(std::slice::from_ref(&report), &private, None)
            .await
            .unwrap();
        std::fs::remove_file(&report).unwrap();
        std::os::unix::fs::symlink(secrets.join("id_key"), &report).unwrap();
        assert_eq!(
            bytes_if_unchanged(&read[0]).await.as_deref(),
            Some(&b"report bytes"[..])
        );
    }

    /// A hard link to a private file has an innocent path: a file with other names is refused.
    #[tokio::test]
    async fn a_hard_link_is_refused() {
        let dir = folder();
        let secrets = dir.path().join("secrets");
        std::fs::create_dir(&secrets).unwrap();
        std::fs::write(secrets.join("id_key"), b"private key bytes").unwrap();
        let private = PrivatePaths::new([secrets.clone()]);
        let innocent = dir.path().join("notes.txt");
        std::fs::hard_link(secrets.join("id_key"), &innocent).unwrap();
        assert!(matches!(
            read_all(&[innocent], &private, None).await,
            Err(FileRefused::Linked(_))
        ));
    }

    #[tokio::test]
    async fn an_agents_file_must_be_in_its_roots_as_its_real_path() {
        let dir = folder();
        let root = dir.path().join("agents").join("client-a");
        std::fs::create_dir_all(&root).unwrap();
        let mine = root.join("brief.pdf");
        std::fs::write(&mine, b"%PDF example").unwrap();
        let owners = dir.path().join("contract.pdf");
        std::fs::write(&owners, b"%PDF the owner's").unwrap();
        let private = PrivatePaths::new(Vec::<PathBuf>::new());
        let roots = vec![root.clone()];
        assert!(read_all(&[mine], &private, Some(&roots)).await.is_ok());
        assert!(matches!(
            read_all(std::slice::from_ref(&owners), &private, Some(&roots)).await,
            Err(FileRefused::Outside(_))
        ));
        #[cfg(unix)]
        {
            let link = root.join("innocent.pdf");
            std::os::unix::fs::symlink(&owners, &link).unwrap();
            assert!(
                matches!(
                    read_all(&[link], &private, Some(&roots)).await,
                    Err(FileRefused::Outside(_))
                ),
                "a link in its folder to a file outside"
            );
        }
        assert!(
            read_all(&[owners], &private, None).await.is_ok(),
            "the owner's own steps"
        );
    }
}
