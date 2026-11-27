//! The pictures and files the user adds to a message for a session (or a new session's first
//! one), as a GPUI entity its box holds: each stored on the server as it is picked, then sent
//! with the message.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

use emberhut_api::chat::ChatFile;
use gpui_kit::{AppContext as _, Context};

use crate::backend::{Backend, UploadSource};

/// How many files a message takes.
pub(crate) const MOST: usize = 10;

/// The files of one box.
pub(crate) struct SessionFiles {
    backend: Arc<dyn Backend>,
    files: Vec<ChatFile>,
    /// How many are still being stored.
    uploading: usize,
    /// Why one didn't store.
    note: Option<String>,
}

impl SessionFiles {
    /// An empty box's files.
    #[must_use]
    pub(crate) fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            files: Vec::new(),
            uploading: 0,
            note: None,
        }
    }

    /// The files stored so far.
    #[must_use]
    pub(crate) fn files(&self) -> &[ChatFile] {
        &self.files
    }

    /// Whether some are still being stored (the message waits for them).
    #[must_use]
    pub(crate) fn is_uploading(&self) -> bool {
        self.uploading > 0
    }

    /// Why a file didn't store.
    #[must_use]
    pub(crate) fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Stores the files at `paths` (as many as there is room for).
    pub(crate) fn add(&mut self, paths: Vec<PathBuf>, cx: &mut Context<'_, Self>) {
        let room = MOST.saturating_sub(self.files.len() + self.uploading);
        self.note = None;
        for path in paths.into_iter().take(room) {
            self.uploading += 1;
            let name = path.file_name().map_or_else(
                || "file".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            let backend = Arc::clone(&self.backend);
            cx.spawn(async move |this, cx| {
                let stored = cx
                    .background_spawn(async move {
                        backend.upload(&name, &UploadSource::File(path), &mut |_| {
                            ControlFlow::Continue(())
                        })
                    })
                    .await;
                this.update(cx, |files, cx| {
                    files.uploading = files.uploading.saturating_sub(1);
                    match stored {
                        Ok(file) => files.files.push(file),
                        Err(error) => {
                            log::warn!("a file for a session didn't upload: {error}");
                            files.note = Some("A file didn't upload; try again".to_owned());
                        }
                    }
                    cx.notify();
                })
                .inspect_err(|error| log::debug!("the box closed: {error}"))
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Takes file `index` off.
    pub(crate) fn remove(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if index < self.files.len() {
            self.files.remove(index);
            cx.notify();
        }
    }

    /// Lets go of every file: the message on its way took them.
    pub(crate) fn clear(&mut self, cx: &mut Context<'_, Self>) {
        self.note = None;
        self.files.clear();
        cx.notify();
    }

    /// Puts files back after a message didn't go.
    pub(crate) fn put_back(&mut self, mut files: Vec<ChatFile>, cx: &mut Context<'_, Self>) {
        files.append(&mut self.files);
        self.files = files;
        cx.notify();
    }
}
