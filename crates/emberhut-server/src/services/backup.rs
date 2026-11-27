//! The server's backup (`GET /api/backup`): the server's data, as one `.tar.gz` streamed while
//! it is made.
//!
//! The archive holds:
//!
//! - `emberhut.db`: a consistent snapshot of the database, made by SQLite while the server runs
//!   ([`BackupStore::snapshot_into`]): one moment of it, with the commits that are still only in
//!   the write-ahead log;
//! - the entries of [`DATA_ENTRIES`] that exist in the server's data folder (the database's
//!   folder), each a file or a folder copied whole.
//!
//! It does **not** hold the secrets directory (`--secrets-dir`: the push services' keys and the
//! secrets saved through one-time links). A backup is a download any signed-in device can
//! make, and it would then carry every key the server holds; the secrets are few, each can be
//! made again where it came from, and they are kept and restored on their own.
//!
//! One backup runs at a time ([`BackupError::Busy`] while one does): each makes a full copy of
//! the database, and several at once could fill the disk. The snapshot is made first, in a
//! private folder of its own in the data folder (`.emberhut-backup-*`, owner-only, deleted after),
//! so it lands on the database's own disk rather than in a small system temp folder, and a
//! failure there answers 500 before anything is sent. A folder left there by a backup a crash
//! cut short is removed when the next backup starts. The archive is then written on a blocking
//! thread, file by file, straight into the response: nothing but the snapshot is staged. Only
//! regular files and folders go in. A symlink, socket, FIFO or device is skipped unopened (a
//! FIFO would wait for a writer for ever), and so is anything that cannot be read; each skip is
//! logged, and one bad file never costs the whole backup. A file that changes while it is read
//! goes in at the size it had when it was listed (cut, or padded with zeros, and logged), so the
//! archive stays well formed. A client that goes away, or the server shutting down, stops the
//! work; after the answer has started, a failure can only show as a cut-off archive.

use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Bytes;
use emberhut_core::Clock;
use emberhut_db::{BackupStore, DbError};
use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::Stream;
use jiff::Timestamp;
use tar::{EntryType, Header, HeaderMode};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

/// What the server keeps next to its database, by name, and backs up when it is there. A part
/// that keeps data in a new file or folder of the data folder adds its name here.
pub const DATA_ENTRIES: &[&str] = &["uploads", "files", "context", "calls", "browser", "mac"];

/// The database snapshot's name, in the archive and in the private folder.
pub const DATABASE_MEMBER: &str = "emberhut.db";

/// How the private folders for a snapshot start, in the data folder.
pub const STAGING_PREFIX: &str = ".emberhut-backup-";

/// How many bytes go to the client at a time.
const CHUNK: usize = 64 * 1024;

/// How many chunks wait for a slow client before the writer waits too.
const QUEUE: usize = 8;

/// Why a backup could not start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BackupError {
    /// A backup is being made already (503): one runs at a time.
    #[error("a backup is being made already")]
    Busy,
    /// The private folder for the snapshot could not be made.
    #[error("could not make a private folder for the backup")]
    Staging(#[source] io::Error),
    /// The database could not be snapshotted.
    #[error("could not snapshot the database")]
    Snapshot(#[source] DbError),
}

/// Makes backups.
#[derive(Clone, Debug)]
pub struct BackupService {
    store: BackupStore,
    data_dir: PathBuf,
    clock: Arc<dyn Clock>,
    stopping: CancellationToken,
    running: Arc<Semaphore>,
}

impl BackupService {
    /// Backups of the database `store` reaches and of the data folder it is in, telling the
    /// time with `clock`; a backup in progress stops when `stopping` is cancelled.
    #[must_use]
    pub fn new(store: BackupStore, clock: Arc<dyn Clock>, stopping: CancellationToken) -> Self {
        let data_dir = match store.file().parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
        Self {
            store,
            data_dir,
            clock,
            stopping,
            running: Arc::new(Semaphore::new(1)),
        }
    }

    /// The archive's file name for a backup made now: `emberhut-backup-2026-11-26.tar.gz`.
    #[must_use]
    pub fn file_name(&self) -> String {
        file_name(self.clock.now())
    }

    /// Snapshots the database, then starts writing the archive: its bytes come from the
    /// returned stream, as they are made.
    ///
    /// # Errors
    ///
    /// [`BackupError::Busy`] while another backup runs, or another [`BackupError`] when the
    /// snapshot could not be made.
    pub async fn start(&self) -> Result<BackupStream, BackupError> {
        // Held by the writer and by the stream: the backup runs until both are done, the
        // archive written and the client gone or served.
        let running = Arc::new(
            Arc::clone(&self.running)
                .try_acquire_owned()
                .map_err(|_| BackupError::Busy)?,
        );
        let data_dir = self.data_dir.clone();
        let staging = tokio::task::spawn_blocking(move || staging_folder(&data_dir))
            .await
            .map_err(|error| BackupError::Staging(io::Error::other(error)))?
            .map_err(BackupError::Staging)?;
        let snapshot = staging.path().join(DATABASE_MEMBER);
        self.store
            .snapshot_into(&snapshot)
            .await
            .map_err(BackupError::Snapshot)?;
        let (sender, receiver) = mpsc::channel(QUEUE);
        let data_dir = self.data_dir.clone();
        let stopping = self.stopping.clone();
        let writing = Arc::clone(&running);
        tokio::task::spawn_blocking(move || {
            let mut out = ChannelWriter::new(sender, stopping);
            match write_archive(&mut out, &snapshot, &data_dir) {
                Ok(skipped) => tracing::info!(skipped, "backup sent"),
                Err(_) if out.sender.is_closed() => {
                    tracing::info!("the backup's client went away before the end");
                }
                Err(error) => {
                    tracing::warn!(%error, "the backup stopped before the end");
                    // The body ends with this error, so the client sees a cut, not an end.
                    let sent = out.sender.blocking_send(Err(error));
                    if sent.is_err() {
                        tracing::debug!("the backup's client had gone already");
                    }
                }
            }
            drop(staging);
            drop(writing);
        });
        Ok(BackupStream {
            receiver,
            _running: running,
        })
    }
}

/// A new private folder for a snapshot in `data_dir`, once the folders a backup cut short by a
/// crash left there are gone (one backup runs at a time, so any other is a leftover).
fn staging_folder(data_dir: &Path) -> io::Result<tempfile::TempDir> {
    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let leftover = entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_PREFIX)
            && entry.file_type()?.is_dir();
        if leftover {
            match fs::remove_dir_all(entry.path()) {
                Ok(()) => tracing::info!("removed a backup's snapshot a crash left behind"),
                Err(error) => tracing::warn!(%error, "could not remove a leftover backup snapshot"),
            }
        }
    }
    tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(data_dir)
}

/// `emberhut-backup-<UTC day>.tar.gz`.
fn file_name(now: Timestamp) -> String {
    format!("emberhut-backup-{}.tar.gz", now.strftime("%Y-%m-%d"))
}

/// The archive's bytes as they are made; it ends with an error when the backup was cut short.
#[derive(Debug)]
pub struct BackupStream {
    receiver: mpsc::Receiver<io::Result<Bytes>>,
    /// The backup's turn, given back when the stream is dropped (and the writer is done).
    _running: Arc<OwnedSemaphorePermit>,
}

impl Stream for BackupStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

/// Writes the archive to `out`: the snapshot, then the data folder's entries. Returns how many
/// entries were skipped.
fn write_archive(out: &mut dyn Write, snapshot: &Path, data_dir: &Path) -> io::Result<usize> {
    let mut archive = tar::Builder::new(GzEncoder::new(out, Compression::default()));
    let mut skipped = 0;
    let meta = fs::metadata(snapshot)?;
    append_file(
        &mut archive,
        File::open(snapshot)?,
        &meta,
        Path::new(DATABASE_MEMBER),
    )?;
    for name in DATA_ENTRIES {
        let path = data_dir.join(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) => append_entry(&mut archive, &path, &meta, Path::new(name), &mut skipped)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => skip(&path, &error.to_string(), &mut skipped),
        }
    }
    archive.into_inner()?.finish()?.flush()?;
    Ok(skipped)
}

type Archive<'a> = tar::Builder<GzEncoder<&'a mut dyn Write>>;

/// Adds `path` (whose own metadata is `meta`, links not followed) as `name`: a folder with
/// everything in it, in name order, or a regular file. Anything else, or anything that can't
/// be read, is skipped and counted. Only writing the archive fails this.
fn append_entry(
    archive: &mut Archive<'_>,
    path: &Path,
    meta: &Metadata,
    name: &Path,
    skipped: &mut usize,
) -> io::Result<()> {
    if meta.is_dir() {
        let mut children =
            match fs::read_dir(path).and_then(Iterator::collect::<io::Result<Vec<_>>>) {
                Ok(children) => children,
                Err(error) => {
                    skip(path, &error.to_string(), skipped);
                    return Ok(());
                }
            };
        children.sort_by_key(fs::DirEntry::file_name);
        let mut header = Header::new_gnu();
        header.set_metadata_in_mode(meta, HeaderMode::Complete);
        header.set_entry_type(EntryType::Directory);
        header.set_size(0);
        archive.append_data(&mut header, name, io::empty())?;
        for child in children {
            let child_path = child.path();
            match fs::symlink_metadata(&child_path) {
                Ok(child_meta) => append_entry(
                    archive,
                    &child_path,
                    &child_meta,
                    &name.join(child.file_name()),
                    skipped,
                )?,
                Err(error) => skip(&child_path, &error.to_string(), skipped),
            }
        }
    } else if meta.is_file() {
        match open_same(path, meta) {
            Ok(file) => {
                if let Some(problem) = append_file(archive, file, meta, name)? {
                    skip(path, &problem, skipped);
                }
            }
            Err(problem) => skip(path, &problem, skipped),
        }
    } else {
        skip(path, "not a regular file or a folder", skipped);
    }
    Ok(())
}

/// Opens the regular file at `path`, if it is still the one `meta` describes (not swapped for
/// another file or a link since it was listed).
fn open_same(path: &Path, meta: &Metadata) -> Result<File, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if opened.is_file() && same_file(&opened, meta) {
        Ok(file)
    } else {
        Err("changed while the backup listed it".to_owned())
    }
}

#[cfg(unix)]
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_: &Metadata, _: &Metadata) -> bool {
    true
}

/// Adds `file` as `name`, at the size `meta` gives. Returns what went wrong reading it, if
/// anything did: the entry is then cut or padded to that size, so the archive stays whole.
fn append_file(
    archive: &mut Archive<'_>,
    file: File,
    meta: &Metadata,
    name: &Path,
) -> io::Result<Option<String>> {
    let mut header = Header::new_gnu();
    header.set_metadata_in_mode(meta, HeaderMode::Complete);
    header.set_entry_type(EntryType::Regular);
    let mut body = ExactLength::new(file, meta.len());
    archive.append_data(&mut header, name, &mut body)?;
    Ok(body.problem)
}

/// Logs a skipped entry and counts it.
fn skip(path: &Path, why: &str, skipped: &mut usize) {
    tracing::warn!(path = %path.display(), why, "backup: skipped");
    *skipped += 1;
}

/// Exactly `left` bytes of a file: its own while they last (cut there when it has grown), then
/// zeros when it has shrunk or a read fails, with the reason kept.
struct ExactLength<R> {
    inner: R,
    left: u64,
    problem: Option<String>,
}

impl<R: Read> ExactLength<R> {
    fn new(inner: R, len: u64) -> Self {
        Self {
            inner,
            left: len,
            problem: None,
        }
    }
}

impl<R: Read> Read for ExactLength<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let want = usize::try_from(self.left).map_or(buf.len(), |left| left.min(buf.len()));
        if want == 0 {
            return Ok(0);
        }
        let buf = &mut buf[..want];
        let got = if self.problem.is_some() {
            0
        } else {
            loop {
                match self.inner.read(buf) {
                    Ok(0) => {
                        self.problem = Some("shrank while it was read; padded".to_owned());
                        break 0;
                    }
                    Ok(got) => break got,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        self.problem = Some(format!("{error}; the rest padded"));
                        break 0;
                    }
                }
            }
        };
        let got = if got == 0 {
            buf.fill(0);
            want
        } else {
            got
        };
        self.left -= got as u64;
        Ok(got)
    }
}

/// Sends what is written to the response in chunks; fails once the client has gone or the
/// server is stopping.
struct ChannelWriter {
    sender: mpsc::Sender<io::Result<Bytes>>,
    stopping: CancellationToken,
    buffer: Vec<u8>,
}

impl ChannelWriter {
    fn new(sender: mpsc::Sender<io::Result<Bytes>>, stopping: CancellationToken) -> Self {
        Self {
            sender,
            stopping,
            buffer: Vec::with_capacity(CHUNK),
        }
    }

    fn send(&mut self) -> io::Result<()> {
        if self.stopping.is_cancelled() {
            return Err(io::Error::other("the server is shutting down"));
        }
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::replace(
            &mut self.buffer,
            Vec::with_capacity(CHUNK),
        ));
        self.sender
            .blocking_send(Ok(chunk))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the client went away"))
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let room = CHUNK - self.buffer.len();
        let taken = data.len().min(room);
        self.buffer.extend_from_slice(&data[..taken]);
        if self.buffer.len() >= CHUNK {
            self.send()?;
        }
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn read_all(reader: &mut impl Read) -> Vec<u8> {
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn exact_length_cuts_a_grown_file_and_pads_a_shrunk_one() {
        let mut grown = ExactLength::new(Cursor::new(b"abcdef".to_vec()), 4);
        assert_eq!(read_all(&mut grown), b"abcd");
        assert_eq!(grown.problem, None);

        let mut shrunk = ExactLength::new(Cursor::new(b"ab".to_vec()), 5);
        assert_eq!(read_all(&mut shrunk), b"ab\0\0\0");
        assert!(shrunk.problem.unwrap().starts_with("shrank"));
    }

    #[test]
    fn exact_length_pads_after_a_failed_read() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk error"))
            }
        }
        let mut failing = ExactLength::new(Failing, 3);
        assert_eq!(read_all(&mut failing), b"\0\0\0");
        assert_eq!(
            failing.problem.as_deref(),
            Some("disk error; the rest padded")
        );
    }

    #[test]
    fn the_file_name_is_the_utc_day() {
        let at: Timestamp = "2026-11-26T23:30:00Z".parse().unwrap();
        assert_eq!(file_name(at), "emberhut-backup-2026-11-26.tar.gz");
    }

    #[test]
    fn the_writer_stops_once_the_server_is_stopping() {
        let (sender, _receiver) = mpsc::channel(QUEUE);
        let stopping = CancellationToken::new();
        let mut writer = ChannelWriter::new(sender, stopping.clone());
        writer.write_all(b"some").unwrap();
        writer.flush().unwrap();
        stopping.cancel();
        assert!(writer.flush().is_err());
    }

    #[test]
    fn the_writer_fails_once_the_client_has_gone() {
        let (sender, receiver) = mpsc::channel(QUEUE);
        drop(receiver);
        let mut writer = ChannelWriter::new(sender, CancellationToken::new());
        writer.write_all(b"some").unwrap();
        let error = writer.flush().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
