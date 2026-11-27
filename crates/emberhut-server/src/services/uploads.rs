//! The chat's files: storing what the person attaches (`POST /api/uploads`), finding a stored
//! file to serve (`GET /uploads/<stored name>`), and checking the files a message comes with.
//!
//! The files live in the uploads folder, `uploads/` beside the database file; each is noted in
//! the `files` table with its name as given and its kind. A stored name is 8 random hex digits,
//! `-`, and the name made safe (`emberhut_core::upload`), so names never collide and never leave
//! the folder. A file sent back with a message is taken only when its
//! `path` is in the uploads folder, the file is there, and the server noted it; anything else is
//! dropped.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use emberhut_api as api;
use emberhut_api::chat::ChatFile;
use emberhut_api::uploads::MAX_SIZE;
use emberhut_core::upload::{
    self, FileKind, NewFile, StoredFile, UNKNOWN_TYPE, is_pulled_name, is_stored_name, stored_name,
    url_of,
};
use emberhut_core::{Clock, RandomError, RandomSource};
use emberhut_db::{DbError, FileStore};
use futures_util::{Stream, StreamExt as _};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::services::private::PrivatePaths;

/// The folder of the chat's files, beside the database file.
pub const FOLDER: &str = "uploads";

/// The longest wait for the next part of an upload's body: an upload has no overall time
/// limit (a phone video takes minutes), but one that stops arriving is given up.
pub const IDLE: Duration = Duration::from_secs(60);

/// The name a file without a usable one is shown and stored as.
const UNNAMED: &str = "file";

/// Why a file was not stored.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UploadError {
    /// The file is over [`MAX_SIZE`]: by its `Content-Length`, before anything is read, or
    /// once more than that has arrived.
    #[error("file is larger than 500 MB")]
    TooLarge,
    /// The body stopped arriving, or failed, before its end.
    #[error("the file did not arrive whole")]
    Interrupted(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The file could not be written to the uploads folder.
    #[error("the file could not be written")]
    Storage(#[source] io::Error),
    /// No randomness for its stored name.
    #[error("no randomness for a stored name")]
    Random(#[from] RandomError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The file is one that never leaves the server: a secret, a key or token file, the
    /// database or its journals, an agent account's configuration.
    #[error("that file never leaves the server")]
    Private,
    /// The file is in the uploads folder already: the chat's own files are not kept again.
    #[error("that file is one of the chat's files already")]
    InUploads,
    /// The file, or a folder it is in, is hidden (its name starts with a dot): where keys,
    /// tokens and tools' configuration live.
    #[error("that file is hidden, or in a hidden folder")]
    Hidden,
}

/// A file of a message, for the assistant: what the chat shows of it and where it is on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// The file as stored.
    pub file: StoredFile,
    /// Where it is.
    pub path: PathBuf,
}

/// A stored file, opened to be served.
#[derive(Debug)]
pub struct Served {
    /// The open file.
    pub file: tokio::fs::File,
    /// Its size in bytes.
    pub size: u64,
    /// The type it is served with, from its name's extension.
    pub content_type: &'static str,
}

/// Stores, finds and checks the chat's files.
#[derive(Clone, Debug)]
pub struct UploadService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    files: FileStore,
    dir: PathBuf,
    private: PrivatePaths,
    clock: Arc<dyn Clock>,
    random: Arc<dyn RandomSource>,
}

impl UploadService {
    /// A service keeping files in `dir` (made when the first file comes), noting them in
    /// `files`; it never keeps a file of `private` ([`Self::keep`]).
    #[must_use]
    pub fn new(
        files: FileStore,
        dir: PathBuf,
        private: PrivatePaths,
        clock: Arc<dyn Clock>,
        random: Arc<dyn RandomSource>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                files,
                dir,
                private,
                clock,
                random,
            }),
        }
    }

    /// The uploads folder beside the database file at `database`, as an absolute path (a
    /// file's `path` is compared against it).
    #[must_use]
    pub fn folder_beside(database: &Path) -> PathBuf {
        let folder = database
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(FOLDER);
        std::path::absolute(&folder).unwrap_or(folder)
    }

    /// The uploads folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Where a stored file is on disk.
    #[must_use]
    pub fn path_of(&self, file: &StoredFile) -> PathBuf {
        self.inner.dir.join(&file.stored_name)
    }

    /// Stores the file called `name` whose bytes are `body` (of `length` bytes, when the request
    /// said), and notes it. A blank name is stored and shown as `file`.
    ///
    /// The body is written as it arrives, never held in memory; a part of a file is never kept.
    ///
    /// # Errors
    ///
    /// [`UploadError::TooLarge`] for a file over [`MAX_SIZE`] (by `length`, before the body is
    /// read), [`UploadError::Interrupted`] when the body fails or nothing arrives for [`IDLE`],
    /// [`UploadError::Storage`], [`UploadError::Random`] and [`UploadError::Database`].
    pub async fn receive<S, E>(
        &self,
        name: &str,
        length: Option<u64>,
        body: S,
    ) -> Result<StoredFile, UploadError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        if length.is_some_and(|length| length > MAX_SIZE) {
            return Err(UploadError::TooLarge);
        }
        let name = if name.trim().is_empty() {
            UNNAMED
        } else {
            name
        };
        let stored = stored_name(self.inner.random.as_ref(), name)?;
        tokio::fs::create_dir_all(&self.inner.dir)
            .await
            .map_err(UploadError::Storage)?;
        let path = self.inner.dir.join(&stored);
        let written = write_new(&path, body).await;
        let size = match written {
            Ok(size) => size,
            Err(error) => {
                remove(&path).await;
                return Err(error);
            }
        };
        let noted = self
            .inner
            .files
            .add(
                NewFile {
                    stored_name: stored,
                    name: name.to_owned(),
                    kind: FileKind::of(name),
                    size,
                },
                self.inner.clock.now(),
            )
            .await;
        if noted.is_err() {
            remove(&path).await;
        }
        Ok(noted?)
    }

    /// Copies the file at `source` (one the server sends in the chat: a chart, a report) into
    /// the uploads folder and notes it, under its own file name.
    ///
    /// The path is resolved to its real path first, and the file is refused when that is
    /// private (the set the service was made with), in the uploads folder, or hidden (a name
    /// on the way starting with a dot), so no link or `..` takes a private file out. It is
    /// then opened once, and what is copied is read from that handle, at most [`MAX_SIZE`]:
    /// the size checked is the size copied, however the file changes meanwhile.
    ///
    /// # Errors
    ///
    /// [`UploadError::Storage`] when there is no file there or it can't be copied,
    /// [`UploadError::Private`], [`UploadError::InUploads`] and [`UploadError::Hidden`] for a
    /// file that is not kept, [`UploadError::TooLarge`] for one over [`MAX_SIZE`],
    /// [`UploadError::Random`] and [`UploadError::Database`].
    pub async fn keep(&self, source: &Path) -> Result<StoredFile, UploadError> {
        let real = tokio::fs::canonicalize(source)
            .await
            .map_err(UploadError::Storage)?;
        tokio::fs::create_dir_all(&self.inner.dir)
            .await
            .map_err(UploadError::Storage)?;
        let uploads = tokio::fs::canonicalize(&self.inner.dir)
            .await
            .map_err(UploadError::Storage)?;
        if self.inner.private.holds(&real) {
            return Err(UploadError::Private);
        }
        if real.starts_with(&uploads) {
            return Err(UploadError::InUploads);
        }
        if is_hidden(&real) {
            return Err(UploadError::Hidden);
        }
        let mut from = tokio::fs::File::open(&real)
            .await
            .map_err(UploadError::Storage)?;
        let meta = from.metadata().await.map_err(UploadError::Storage)?;
        if !meta.is_file() {
            return Err(UploadError::Storage(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a file",
            )));
        }
        if meta.len() > MAX_SIZE {
            return Err(UploadError::TooLarge);
        }
        let name = source.file_name().map_or_else(
            || UNNAMED.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let stored = stored_name(self.inner.random.as_ref(), &name)?;
        let path = self.inner.dir.join(&stored);
        let size = match copy_new(&mut from, &path).await {
            Ok(size) => size,
            Err(error) => {
                remove(&path).await;
                return Err(error);
            }
        };
        let noted = self
            .inner
            .files
            .add(
                NewFile {
                    stored_name: stored,
                    kind: FileKind::of(&name),
                    name,
                    size,
                },
                self.inner.clock.now(),
            )
            .await;
        if noted.is_err() {
            remove(&path).await;
        }
        Ok(noted?)
    }

    /// The stored file a message's `file` names, when the server takes it: its `path` is
    /// directly in the uploads folder, the file is there, and it was noted. `None` otherwise.
    ///
    /// # Errors
    ///
    /// [`UploadError::Database`].
    pub async fn known(&self, file: &ChatFile) -> Result<Option<StoredFile>, UploadError> {
        let Some(stored_name) = file
            .path
            .as_deref()
            .and_then(|path| self.stored_name_of(Path::new(path)))
        else {
            return Ok(None);
        };
        let Some(stored) = self.inner.files.by_stored_name(stored_name).await? else {
            return Ok(None);
        };
        let there = tokio::fs::symlink_metadata(self.path_of(&stored))
            .await
            .is_ok_and(|meta| meta.is_file());
        Ok(there.then_some(stored))
    }

    /// The files of `sent` the server takes (see [`Self::known`]), in order, each with where
    /// it is; the others are dropped.
    ///
    /// # Errors
    ///
    /// [`UploadError::Database`].
    pub async fn attachments(&self, sent: &[ChatFile]) -> Result<Vec<Attachment>, UploadError> {
        let mut taken = Vec::with_capacity(sent.len());
        for file in sent {
            if let Some(file) = self.known(file).await? {
                taken.push(Attachment {
                    path: self.path_of(&file),
                    file,
                });
            }
        }
        Ok(taken)
    }

    /// The stored name of `path` when it is a file directly in the uploads folder.
    fn stored_name_of<'a>(&self, path: &'a Path) -> Option<&'a str> {
        let rest = path.strip_prefix(&self.inner.dir).ok()?;
        let mut parts = rest.components();
        let (Some(Component::Normal(name)), None) = (parts.next(), parts.next()) else {
            return None;
        };
        name.to_str().filter(|name| is_stored_name(name))
    }

    /// The file in the uploads folder called `stored_name`, opened, when there is one. Any file
    /// in the folder is served, noted or not (files copied in by an import are not noted); a
    /// name with `..` or a separator in it never is, and a symbolic link is not followed out.
    ///
    /// # Errors
    ///
    /// The I/O error when the file is there but can't be opened.
    pub async fn open(&self, stored_name: &str) -> io::Result<Option<Served>> {
        let pulled = is_pulled_name(stored_name);
        if !is_stored_name(stored_name) && !pulled {
            return Ok(None);
        }
        let path = self.inner.dir.join(stored_name);
        // A brought file's folder is a real folder in the uploads folder, never a link out.
        if pulled
            && !tokio::fs::symlink_metadata(path.parent().unwrap_or(&self.inner.dir))
                .await
                .is_ok_and(|meta| meta.is_dir())
        {
            return Ok(None);
        }
        let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
            return Ok(None);
        };
        if !meta.is_file() {
            return Ok(None);
        }
        let file = tokio::fs::File::open(&path).await?;
        Ok(Some(Served {
            file,
            size: meta.len(),
            content_type: upload::content_type(stored_name).unwrap_or(UNKNOWN_TYPE),
        }))
    }

    /// The file as `POST /api/uploads` answers it: with its `path` on the server, which the
    /// client sends back with the message.
    #[must_use]
    pub fn reply_file(&self, file: &StoredFile) -> ChatFile {
        ChatFile {
            path: Some(self.path_of(file).to_string_lossy().into_owned()),
            ..chat_file(file)
        }
    }
}

/// A stored file as a chat message shows it: name, `url` and kind, without its path.
#[must_use]
pub fn chat_file(file: &StoredFile) -> ChatFile {
    ChatFile {
        name: file.name.clone(),
        url: url_of(&file.stored_name),
        kind: api::chat::FileKind::from(file.kind.as_str()),
        path: None,
    }
}

/// The lines under the person's message that tell the assistant about its files: how many,
/// and each one's kind and path (pictures to read; a video comes with still frames the app
/// took). Empty without files.
#[must_use]
pub fn describe(files: &[Attachment]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let plural = if files.len() == 1 { "" } else { "s" };
    let mut text = format!(
        "\n\nThe person attached {} file{plural} (read pictures with your Read tool; a video \
         comes with still frames, the pictures named \"… frame …\"):",
        files.len()
    );
    for file in files {
        text.push_str("\n- ");
        text.push_str(file.file.kind.as_str());
        text.push_str(": ");
        text.push_str(&file.path.to_string_lossy());
    }
    text
}

/// Writes `body` to a new file at `path`, and gives its size.
async fn write_new<S, E>(path: &Path, mut body: S) -> Result<u64, UploadError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(UploadError::Storage)?;
    let mut size = 0_u64;
    loop {
        let next = tokio::time::timeout(IDLE, body.next())
            .await
            .map_err(|elapsed| UploadError::Interrupted(Box::new(elapsed)))?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|error| UploadError::Interrupted(Box::new(error)))?;
        size += u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        if size > MAX_SIZE {
            return Err(UploadError::TooLarge);
        }
        file.write_all(&chunk).await.map_err(UploadError::Storage)?;
    }
    file.flush().await.map_err(UploadError::Storage)?;
    Ok(size)
}

/// Copies what is left of `from` to a new file at `path`, at most [`MAX_SIZE`] bytes, and
/// gives its size: [`UploadError::TooLarge`] when there is more.
async fn copy_new(from: &mut tokio::fs::File, path: &Path) -> Result<u64, UploadError> {
    let mut to = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(UploadError::Storage)?;
    let size = tokio::io::copy(&mut from.take(MAX_SIZE + 1), &mut to)
        .await
        .map_err(UploadError::Storage)?;
    if size > MAX_SIZE {
        return Err(UploadError::TooLarge);
    }
    to.flush().await.map_err(UploadError::Storage)?;
    Ok(size)
}

/// Whether a part of `path` is hidden: a file or folder whose name starts with a dot.
fn is_hidden(path: &Path) -> bool {
    path.components().any(|part| match part {
        Component::Normal(name) => name.as_encoded_bytes().starts_with(b"."),
        _ => false,
    })
}

/// Removes a file that must not stay (a part of an upload); a failure is only logged.
async fn remove(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), %error, "could not remove a part of a file");
    }
}
