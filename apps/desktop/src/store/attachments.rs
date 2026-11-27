//! The files attached in the composer, before they are sent: each uploads as soon as it is
//! added (as the Apple apps do), shows how far it got, and can fail, be tried again (unless it
//! never can work, as a file over the server's limit) or be removed, which stops its upload.
//! Send waits until none is uploading or still being looked at; it takes the stored files and
//! leaves the composer empty (a failed one is dropped, as there).
//!
//! The sizes of files sent from here are kept by their server address, so the chat can show
//! them (the server's chat files carry none).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use emberhut_api::chat::ChatFile;
use gpui_kit::Image;

use crate::backend::UploadSource;

/// An attachment's identity, for as long as the composer holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AttachmentId(u64);

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where an attachment's upload stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// On its way: `sent` bytes so far.
    Uploading {
        /// The bytes sent.
        sent: u64,
    },
    /// Stored on the server: this goes with the message.
    Ready(ChatFile),
    /// It failed.
    Failed {
        /// Why, worded for the user.
        reason: String,
        /// Whether trying again may work (not for a file over the server's limit).
        retryable: bool,
    },
}

/// Tells a running upload to stop: set when its attachment is removed.
#[derive(Clone, Debug, Default)]
pub(crate) struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    /// Whether the upload should stop.
    #[must_use]
    pub(crate) fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A file in the composer.
#[derive(Clone)]
pub(crate) struct Attachment {
    /// Its identity.
    pub(crate) id: AttachmentId,
    /// Its name, as stored and shown.
    pub(crate) name: String,
    /// Its size in bytes.
    pub(crate) size: u64,
    /// What the upload reads.
    pub(crate) source: UploadSource,
    /// A pasted picture, to show it (its bytes are the ones the upload reads; a file on disk is
    /// shown from its path).
    pub(crate) picture: Option<Arc<Image>>,
    /// Where its upload stands.
    pub(crate) stage: Stage,
    /// The video this is a still frame of: it goes with the video, shows as the video's chip,
    /// and goes when the video is taken out.
    pub(crate) still_of: Option<AttachmentId>,
    /// Stops the upload running now.
    stop: StopFlag,
}

impl fmt::Debug for Attachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachment")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("size", &self.size)
            .field("source", &self.source)
            .field("picture", &self.picture.is_some())
            .field("stage", &self.stage)
            .field("still_of", &self.still_of)
            .finish_non_exhaustive()
    }
}

impl Attachment {
    /// The share uploaded, 0 to 1, while it uploads.
    #[must_use]
    pub(crate) fn progress(&self) -> Option<f32> {
        let Stage::Uploading { sent } = self.stage else {
            return None;
        };
        if self.size == 0 {
            return Some(1.0);
        }
        // Whole permille: f32 is exact for those.
        let permille = sent.min(self.size).saturating_mul(1000) / self.size;
        Some(f32::from(u16::try_from(permille).unwrap_or(1000)) / 1000.0)
    }

    /// The flag that stops its running upload.
    #[must_use]
    pub(crate) fn stop_flag(&self) -> StopFlag {
        self.stop.clone()
    }
}

/// The composer's attachments.
#[derive(Debug, Default)]
pub(crate) struct Attachments {
    items: Vec<Attachment>,
    next: u64,
    /// Files being looked at before they are added (their size read off the disk).
    adding: usize,
    /// The sizes of files sent from here, by their server address.
    sent_sizes: HashMap<String, u64>,
}

impl Attachments {
    /// The attachments, in the order they were added.
    #[must_use]
    pub(crate) fn items(&self) -> &[Attachment] {
        &self.items
    }

    /// The attachments with chips of their own: all but a video's still frames.
    pub(crate) fn shown(&self) -> impl Iterator<Item = &Attachment> {
        self.items.iter().filter(|item| item.still_of.is_none())
    }

    /// The picture a video's chip shows: its first still frame, once it is made.
    #[must_use]
    pub(crate) fn poster(&self, video: AttachmentId) -> Option<&Arc<Image>> {
        self.items
            .iter()
            .filter(|item| item.still_of == Some(video))
            .find_map(|item| item.picture.as_ref())
    }

    /// The attachment `id`, while the composer holds it.
    #[must_use]
    pub(crate) fn get(&self, id: AttachmentId) -> Option<&Attachment> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Files are being looked at; [`Self::added`] says when they are done. Send waits meanwhile.
    pub(crate) fn begin_adding(&mut self) {
        self.adding += 1;
    }

    /// The files [`Self::begin_adding`] announced are added (or left out).
    pub(crate) fn added(&mut self) {
        self.adding = self.adding.saturating_sub(1);
    }

    /// Adds a file whose upload starts now.
    pub(crate) fn add(
        &mut self,
        name: String,
        size: u64,
        source: UploadSource,
        picture: Option<Arc<Image>>,
    ) -> AttachmentId {
        self.push(name, size, source, picture, Stage::Uploading { sent: 0 })
    }

    /// Adds a still frame of `video` whose upload starts now; `None` (and nothing added) when
    /// the video was taken out meanwhile.
    pub(crate) fn add_still(
        &mut self,
        video: AttachmentId,
        name: String,
        size: u64,
        source: UploadSource,
        picture: Arc<Image>,
    ) -> Option<AttachmentId> {
        self.get(video)?;
        let id = self.add(name, size, source, Some(picture));
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.still_of = Some(video);
        }
        Some(id)
    }

    /// Adds a file that can never be uploaded, saying why (it is larger than the server takes).
    pub(crate) fn add_refused(
        &mut self,
        name: String,
        size: u64,
        source: UploadSource,
        reason: String,
    ) -> AttachmentId {
        let stage = Stage::Failed {
            reason,
            retryable: false,
        };
        self.push(name, size, source, None, stage)
    }

    fn push(
        &mut self,
        name: String,
        size: u64,
        source: UploadSource,
        picture: Option<Arc<Image>>,
        stage: Stage,
    ) -> AttachmentId {
        let id = AttachmentId(self.next);
        self.next += 1;
        self.items.push(Attachment {
            id,
            name,
            size,
            source,
            picture,
            stage,
            still_of: None,
            stop: StopFlag::default(),
        });
        id
    }

    fn stage_mut(&mut self, id: AttachmentId) -> Option<&mut Stage> {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .map(|item| &mut item.stage)
    }

    /// `sent` bytes of `id` are out; whether that changed anything.
    pub(crate) fn progress(&mut self, id: AttachmentId, sent: u64) -> bool {
        match self.stage_mut(id) {
            Some(Stage::Uploading { sent: was }) if *was != sent => {
                *was = sent;
                true
            }
            _ => false,
        }
    }

    /// `id`'s upload ended (a failure may be tried again); whether the composer still holds it
    /// uploading (one removed meanwhile is gone for good).
    pub(crate) fn finish(&mut self, id: AttachmentId, outcome: Result<ChatFile, String>) -> bool {
        match self.stage_mut(id) {
            Some(stage @ Stage::Uploading { .. }) => {
                *stage = match outcome {
                    Ok(file) => Stage::Ready(file),
                    Err(reason) => Stage::Failed {
                        reason,
                        retryable: true,
                    },
                };
                true
            }
            _ => false,
        }
    }

    /// Tries a failed upload again, unless it never can work: whether it is uploading now.
    pub(crate) fn retry(&mut self, id: AttachmentId) -> bool {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return false;
        };
        if !matches!(
            item.stage,
            Stage::Failed {
                retryable: true,
                ..
            }
        ) {
            return false;
        }
        item.stage = Stage::Uploading { sent: 0 };
        item.stop = StopFlag::default();
        true
    }

    /// Takes `id` out of the composer (a video with its still frames), and stops each upload
    /// that is running.
    pub(crate) fn remove(&mut self, id: AttachmentId) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        self.items.retain(|item| {
            let goes = item.id == id || item.still_of == Some(id);
            if goes {
                item.stop.set();
            }
            !goes
        });
        true
    }

    /// Whether an upload is still running, or files are still being added.
    #[must_use]
    pub(crate) fn is_uploading(&self) -> bool {
        self.adding > 0
            || self
                .items
                .iter()
                .any(|item| matches!(item.stage, Stage::Uploading { .. }))
    }

    /// Whether a message with `text` can go now: no upload is running, and there is text or a
    /// stored file to send.
    #[must_use]
    pub(crate) fn can_send(&self, text: &str) -> bool {
        let ready = self
            .items
            .iter()
            .any(|item| matches!(item.stage, Stage::Ready(_)));
        !self.is_uploading() && (!text.trim().is_empty() || ready)
    }

    /// The stored files, to send now; the composer is left empty, failed ones included.
    pub(crate) fn take_for_send(&mut self) -> Vec<ChatFile> {
        let mut files = Vec::new();
        for item in self.items.drain(..) {
            if let Stage::Ready(file) = item.stage {
                self.sent_sizes.insert(file.url.clone(), item.size);
                files.push(file);
            }
        }
        files
    }

    /// The size of a file sent from here, by its server address.
    #[must_use]
    pub(crate) fn sent_size(&self, url: &str) -> Option<u64> {
        self.sent_sizes.get(url).copied()
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::chat::FileKind;

    use super::*;

    fn stored(name: &str) -> ChatFile {
        ChatFile {
            name: name.to_owned(),
            url: format!("/uploads/{name}"),
            kind: FileKind::File,
            path: Some(format!("/data/uploads/{name}")),
        }
    }

    fn file(attachments: &mut Attachments, name: &str, size: u64) -> AttachmentId {
        let source = UploadSource::File(format!("/files/{name}").into());
        attachments.add(name.to_owned(), size, source, None)
    }

    fn failed(reason: &str, retryable: bool) -> Stage {
        Stage::Failed {
            reason: reason.to_owned(),
            retryable,
        }
    }

    #[test]
    fn an_upload_goes_from_pending_through_its_progress_to_ready() {
        let mut attachments = Attachments::default();
        let id = file(&mut attachments, "a.pdf", 200);
        let stage = |a: &Attachments| a.get(id).map(|item| item.stage.clone());
        assert_eq!(stage(&attachments), Some(Stage::Uploading { sent: 0 }));
        assert_eq!(
            attachments.get(id).and_then(Attachment::progress),
            Some(0.0)
        );
        assert!(attachments.progress(id, 50));
        assert!(!attachments.progress(id, 50), "nothing new");
        assert_eq!(
            attachments.get(id).and_then(Attachment::progress),
            Some(0.25)
        );
        assert!(attachments.finish(id, Ok(stored("a.pdf"))));
        assert_eq!(stage(&attachments), Some(Stage::Ready(stored("a.pdf"))));
        assert_eq!(attachments.get(id).and_then(Attachment::progress), None);
        assert!(!attachments.progress(id, 60), "done is done");
        assert!(!attachments.retry(id), "only a failure is tried again");
    }

    #[test]
    fn a_failure_is_tried_again_from_the_start() {
        let mut attachments = Attachments::default();
        let id = file(&mut attachments, "a.pdf", 200);
        attachments.progress(id, 120);
        assert!(attachments.finish(id, Err("Can't reach the server".into())));
        assert_eq!(
            attachments.get(id).map(|item| &item.stage),
            Some(&failed("Can't reach the server", true))
        );
        assert!(!attachments.is_uploading());
        assert!(attachments.retry(id));
        assert_eq!(
            attachments.get(id).map(|item| &item.stage),
            Some(&Stage::Uploading { sent: 0 })
        );
        assert!(attachments.finish(id, Ok(stored("a.pdf"))));
    }

    #[test]
    fn removing_stops_the_upload_and_it_finishes_unseen() {
        let mut attachments = Attachments::default();
        let id = file(&mut attachments, "a.pdf", 200);
        let stop = attachments.get(id).map(Attachment::stop_flag).unwrap();
        assert!(!stop.is_set());
        assert!(attachments.remove(id));
        assert!(stop.is_set(), "the running upload is told to stop");
        assert!(!attachments.remove(id));
        assert!(!attachments.finish(id, Ok(stored("a.pdf"))));
        assert!(attachments.items().is_empty());
    }

    #[test]
    fn a_retry_runs_with_a_fresh_stop_flag() {
        let mut attachments = Attachments::default();
        let id = file(&mut attachments, "a.pdf", 200);
        let first = attachments.get(id).map(Attachment::stop_flag).unwrap();
        attachments.finish(id, Err("Can't reach the server".into()));
        attachments.retry(id);
        let second = attachments.get(id).map(Attachment::stop_flag).unwrap();
        attachments.remove(id);
        assert!(second.is_set());
        assert!(!first.is_set(), "the earlier attempt had ended");
    }

    #[test]
    fn send_waits_for_uploads_and_takes_the_stored_files() {
        let mut attachments = Attachments::default();
        assert!(!attachments.can_send("  "));
        assert!(attachments.can_send("Hello"));
        let (a, b, c) = (
            file(&mut attachments, "a.pdf", 10),
            file(&mut attachments, "b.png", 20),
            file(&mut attachments, "c.txt", 30),
        );
        assert!(!attachments.can_send("Hello"), "a.pdf is uploading");
        attachments.finish(a, Ok(stored("a.pdf")));
        attachments.finish(b, Ok(stored("b.png")));
        assert!(!attachments.can_send(""), "c.txt is uploading");
        attachments.finish(c, Err("Too large".into()));
        assert!(attachments.can_send(""), "the stored files go without text");
        assert_eq!(
            attachments.take_for_send(),
            [stored("a.pdf"), stored("b.png")]
        );
        assert!(attachments.items().is_empty(), "the failed one is dropped");
        assert_eq!(attachments.sent_size("/uploads/b.png"), Some(20));
        assert_eq!(attachments.sent_size("/uploads/c.txt"), None);
    }

    #[test]
    fn send_waits_while_files_are_being_added() {
        let mut attachments = Attachments::default();
        attachments.begin_adding();
        assert!(!attachments.can_send("Hello"));
        attachments.added();
        assert!(attachments.can_send("Hello"));
        attachments.added();
        assert!(attachments.can_send("Hello"), "never below none");
    }

    #[test]
    fn a_file_over_the_limit_is_refused_for_good() {
        let mut attachments = Attachments::default();
        let source = UploadSource::File("/files/big.mov".into());
        let id = attachments.add_refused("big.mov".into(), 9, source, "Too large".into());
        assert!(!attachments.is_uploading());
        assert!(!attachments.can_send(""));
        assert!(!attachments.retry(id), "trying again can't work");
        assert_eq!(
            attachments.get(id).map(|item| &item.stage),
            Some(&failed("Too large", false)),
            "its reason stays"
        );
        assert_ne!(
            id,
            file(&mut attachments, "a.pdf", 1),
            "ids are never reused"
        );
    }

    #[test]
    fn a_videos_stills_go_with_it_under_its_chip() {
        let mut attachments = Attachments::default();
        let video = file(&mut attachments, "clip.mov", 900);
        let picture = |n: u8| Arc::new(Image::from_bytes(gpui_kit::ImageFormat::Jpeg, vec![n]));
        let still = |a: &mut Attachments, n: u8| {
            let source = UploadSource::Bytes(Arc::new(vec![n]));
            a.add_still(
                video,
                format!("clip.mov · frame {n}.jpg"),
                1,
                source,
                picture(n),
            )
        };
        let first = still(&mut attachments, 1).unwrap();
        still(&mut attachments, 2).unwrap();
        let shown: Vec<_> = attachments.shown().map(|item| item.name.as_str()).collect();
        assert_eq!(shown, ["clip.mov"], "one chip for the video");
        assert_eq!(
            attachments.poster(video).map(|p| p.bytes()),
            Some(&[1_u8][..])
        );
        let stop = attachments.get(first).map(Attachment::stop_flag).unwrap();
        assert!(attachments.remove(video));
        assert!(stop.is_set(), "a still's upload stops with the video");
        assert!(attachments.items().is_empty());
        assert_eq!(
            still(&mut attachments, 3),
            None,
            "no stills for a video taken out"
        );
    }

    #[test]
    fn an_empty_file_is_all_sent() {
        let mut attachments = Attachments::default();
        let id = file(&mut attachments, "empty.txt", 0);
        assert_eq!(
            attachments.get(id).and_then(Attachment::progress),
            Some(1.0)
        );
    }

    #[test]
    fn debug_leaves_a_pictures_bytes_out() {
        let mut attachments = Attachments::default();
        let bytes = vec![7_u8; 4096];
        let picture = Arc::new(Image::from_bytes(gpui_kit::ImageFormat::Png, bytes.clone()));
        let source = UploadSource::Bytes(Arc::new(bytes));
        attachments.add("p.png".into(), 4096, source, Some(picture));
        let shown = format!("{:?}", attachments.items()[0]);
        assert!(shown.contains("picture: true"), "{shown}");
        assert!(shown.len() < 300, "{shown}");
    }
}
