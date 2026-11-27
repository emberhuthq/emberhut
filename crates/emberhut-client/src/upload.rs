//! What [`Client::upload`](crate::Client::upload) sends and how a transfer reports: a file on
//! disk or bytes in memory, under a name, read as it goes out so its progress can be told and
//! the caller can stop it.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use emberhut_api::uploads::MAX_SIZE;

use crate::Error;

/// A file to store on the server for the chat: a file on this device (streamed from disk, never
/// read whole into memory) or bytes already in memory (a pasted picture), under the name the
/// chat shows.
///
/// ```
/// use emberhut_client::Upload;
///
/// let file = Upload::file("reports/Week 3.pdf");
/// assert_eq!(file.name(), "Week 3.pdf");
/// let pasted = Upload::bytes("pasted-1764000000.png", vec![0x89, b'P', b'N', b'G']);
/// assert_eq!(pasted.name(), "pasted-1764000000.png");
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Upload {
    name: String,
    source: Source,
}

#[derive(Clone)]
enum Source {
    File(PathBuf),
    Bytes(Arc<dyn AsRef<[u8]> + Send + Sync>),
}

/// Two sources are the same file, or the same bytes (by their contents).
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(a), Self::File(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => (**a).as_ref() == (**b).as_ref(),
            (Self::File(_), Self::Bytes(_)) | (Self::Bytes(_), Self::File(_)) => false,
        }
    }
}

impl Eq for Source {}

impl Upload {
    /// The file at `path`, named as its last component (`file` when it has none).
    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path.file_name().map_or_else(
            || "file".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        Self {
            name,
            source: Source::File(path),
        }
    }

    /// `bytes`, named `name`. They are kept as given (a shared buffer is not copied).
    #[must_use]
    pub fn bytes(name: impl Into<String>, bytes: impl AsRef<[u8]> + Send + Sync + 'static) -> Self {
        Self {
            name: name.into(),
            source: Source::Bytes(Arc::new(bytes)),
        }
    }

    /// The same upload under another name.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The name it is stored and shown under (a blank one goes as `file`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file it reads, when it is one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            Source::File(path) => Some(path),
            Source::Bytes(_) => None,
        }
    }

    /// Its size now, refused when it is over what the server takes.
    pub(crate) fn size(&self) -> Result<u64, Error> {
        let size = match &self.source {
            Source::File(path) => std::fs::metadata(path).map_err(Error::Read)?.len(),
            Source::Bytes(bytes) => u64::try_from((**bytes).as_ref().len()).unwrap_or(u64::MAX),
        };
        if size > MAX_SIZE {
            return Err(Error::TooLarge {
                size,
                limit: MAX_SIZE,
            });
        }
        Ok(size)
    }

    /// A reader of its first `size` bytes, from the start.
    pub(crate) fn open(&self, size: u64) -> Result<Box<dyn Read + Send + '_>, Error> {
        Ok(match &self.source {
            Source::File(path) => Box::new(File::open(path).map_err(Error::Read)?.take(size)),
            Source::Bytes(bytes) => Box::new((**bytes).as_ref().take(size)),
        })
    }
}

impl fmt::Debug for Upload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Upload");
        debug.field("name", &self.name);
        match &self.source {
            Source::File(path) => debug.field("path", path),
            Source::Bytes(bytes) => debug.field("bytes", &(**bytes).as_ref().len()),
        };
        debug.finish()
    }
}

/// How far a transfer is: `done` bytes of `total` (unknown for a download whose size the
/// server didn't give). Sending again after a renewed session starts over from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// The bytes sent or received so far.
    pub done: u64,
    /// The whole size, when it is known.
    pub total: Option<u64>,
}

impl Progress {
    /// The share done, 0 to 1 (1 for an empty file), when the size is known.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a share for a progress bar: 2^52 bytes is far past any limit here"
    )]
    pub fn fraction(self) -> Option<f64> {
        let total = self.total?;
        Some(if total == 0 {
            1.0
        } else {
            (self.done.min(total) as f64) / (total as f64)
        })
    }
}

/// Why a transfer's own reading or writing stopped (the HTTP library only says the body broke).
#[derive(Debug, Default)]
pub(crate) enum Stopped {
    /// It didn't.
    #[default]
    No,
    /// The caller's progress callback said stop.
    Cancelled,
    /// The local file failed.
    Failed(io::Error),
}

impl Stopped {
    /// The client's error for a transfer that stopped, if it did; `local` wraps a failure of
    /// the local file.
    pub(crate) fn into_error(self, local: fn(io::Error) -> Error) -> Option<Error> {
        match self {
            Self::No => None,
            Self::Cancelled => Some(Error::Cancelled),
            Self::Failed(error) => Some(local(error)),
        }
    }
}

/// A body that tells each read to `report`, stops when `report` says so, and keeps the reason
/// a read of the file failed.
pub(crate) struct Reporting<'a, R> {
    inner: R,
    progress: Progress,
    report: &'a mut dyn FnMut(Progress) -> ControlFlow<()>,
    stopped: &'a mut Stopped,
}

impl<'a, R: Read> Reporting<'a, R> {
    pub(crate) fn new(
        inner: R,
        total: u64,
        report: &'a mut dyn FnMut(Progress) -> ControlFlow<()>,
        stopped: &'a mut Stopped,
    ) -> Self {
        Self {
            inner,
            progress: Progress {
                done: 0,
                total: Some(total),
            },
            report,
            stopped,
        }
    }
}

impl<R: Read> Read for Reporting<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(read) => {
                if read > 0 {
                    self.progress.done += u64::try_from(read).unwrap_or(u64::MAX);
                    if (self.report)(self.progress).is_break() {
                        *self.stopped = Stopped::Cancelled;
                        return Err(io::Error::other("the upload was cancelled"));
                    }
                }
                Ok(read)
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => {
                let told = io::Error::new(error.kind(), error.to_string());
                *self.stopped = Stopped::Failed(error);
                Err(told)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_files_name_is_its_last_component() {
        assert_eq!(Upload::file("/tmp/a/notes.txt").name(), "notes.txt");
        assert_eq!(Upload::file("/").name(), "file");
        let renamed = Upload::file("/tmp/a/notes.txt").named("b.txt");
        assert_eq!(renamed.name(), "b.txt");
        assert_eq!(renamed.path(), Some(Path::new("/tmp/a/notes.txt")));
    }

    #[test]
    fn bytes_are_their_own_size_and_debug_hides_them() {
        let upload = Upload::bytes("a.png", vec![1, 2, 3]);
        assert_eq!(upload.size().unwrap(), 3);
        assert_eq!(upload.path(), None);
        assert_eq!(
            format!("{upload:?}"),
            r#"Upload { name: "a.png", bytes: 3 }"#
        );
    }

    #[test]
    fn uploads_compare_by_name_and_contents() {
        let bytes = Upload::bytes("a.png", vec![1, 2]);
        assert_eq!(
            bytes,
            Upload::bytes("a.png", Arc::<[u8]>::from(&[1_u8, 2][..]))
        );
        assert_ne!(bytes, Upload::bytes("a.png", vec![1, 3]));
        assert_ne!(bytes, bytes.clone().named("b.png"));
        assert_eq!(Upload::file("/tmp/a.png"), Upload::file("/tmp/a.png"));
        assert_ne!(
            Upload::file("/tmp/a.png"),
            Upload::bytes("a.png", vec![1, 2])
        );
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let upload = Upload::file("/nonexistent/emberhut-client/upload.bin");
        assert!(matches!(upload.size(), Err(Error::Read(_))));
    }

    #[test]
    fn progress_is_a_share_when_the_size_is_known() {
        let at = |done, total| Progress { done, total }.fraction();
        assert_eq!(at(1, Some(4)), Some(0.25));
        assert_eq!(at(0, Some(0)), Some(1.0));
        assert_eq!(at(9, Some(4)), Some(1.0));
        assert_eq!(at(9, None), None);
    }

    /// A file that fails to read.
    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disk gone"))
        }
    }

    #[test]
    fn each_read_is_reported_and_a_failure_kept() {
        let mut seen = Vec::new();
        let mut stopped = Stopped::No;
        let mut report = |progress: Progress| {
            seen.push(progress.done);
            ControlFlow::Continue(())
        };
        let mut body = Reporting::new(&b"abcdef"[..], 6, &mut report, &mut stopped);
        let mut buffer = [0; 4];
        assert_eq!(body.read(&mut buffer).unwrap(), 4);
        assert_eq!(body.read(&mut buffer).unwrap(), 2);
        assert_eq!(body.read(&mut buffer).unwrap(), 0);
        assert_eq!(seen, [4, 6]);

        let mut report = |_: Progress| ControlFlow::Continue(());
        let mut body = Reporting::new(Broken, 6, &mut report, &mut stopped);
        assert!(body.read(&mut buffer).is_err());
        let error = stopped.into_error(Error::Read);
        assert!(matches!(error, Some(Error::Read(ref e)) if e.to_string() == "disk gone"));
    }

    #[test]
    fn a_break_stops_the_body() {
        let mut stopped = Stopped::No;
        let mut report = |_: Progress| ControlFlow::Break(());
        let mut body = Reporting::new(&b"abcdef"[..], 6, &mut report, &mut stopped);
        assert!(body.read(&mut [0; 4]).is_err());
        assert!(matches!(
            stopped.into_error(Error::Read),
            Some(Error::Cancelled)
        ));
    }
}
