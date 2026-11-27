//! Attaching files and pictures in the composer: each file is looked at and each picture
//! prepared off the UI thread, then starts uploading at once on a background thread, its
//! progress and its end reported back to the store. Removing it stops the upload.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use emberhut_api::uploads::MAX_SIZE;
use gpui_kit::AppContext as _;
use gpui_kit::{Context, Image};

use super::attachments::{AttachmentId, Attachments};
use super::pasted::Pasted;
use super::{Store, update_or_log};
use crate::backend::{Error, UploadSource};
use crate::format::capitalized;

/// Why a file over the server's limit isn't sent (`MAX_SIZE`, in the chips' units).
const TOO_LARGE: &str = "Larger than the 500 MiB the server takes";

/// A file found at a dropped, pasted or picked path.
struct Found {
    path: PathBuf,
    name: String,
    size: u64,
}

/// The files at `paths`; folders and anything that isn't a readable file are left out, as the
/// Apple apps leave them. It reads the disk: call it off the UI thread.
fn look_at(paths: Vec<PathBuf>) -> Vec<Found> {
    paths
        .into_iter()
        .filter_map(|path| {
            let size = match std::fs::metadata(&path) {
                Ok(meta) if meta.is_file() => meta.len(),
                Ok(_) => {
                    log::info!("not attaching {}: not a file", path.display());
                    return None;
                }
                Err(error) => {
                    log::warn!("not attaching {}: {error}", path.display());
                    return None;
                }
            };
            let name = path.file_name().map_or_else(
                || "file".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            Some(Found { path, name, size })
        })
        .collect()
}

impl Store {
    /// The files attached in the composer.
    #[must_use]
    pub(crate) fn attachments(&self) -> &Attachments {
        &self.attachments
    }

    /// Attaches the files at `paths` (dropped, pasted or picked), once they are looked at off
    /// the UI thread; Send waits meanwhile.
    pub(crate) fn attach_paths(&mut self, paths: &[PathBuf], cx: &mut Context<'_, Self>) {
        let paths = paths.to_vec();
        self.attachments.begin_adding();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { look_at(paths) }).await;
            update_or_log(&this, cx, |store, cx| {
                store.attachments.added();
                for file in found {
                    let source = UploadSource::File(file.path.clone());
                    if file.size > MAX_SIZE {
                        store.attachments.add_refused(
                            file.name,
                            file.size,
                            source,
                            TOO_LARGE.to_owned(),
                        );
                    } else {
                        let video = super::stills::is_video(&file.name);
                        let name = file.name.clone();
                        let id = store.attachments.add(file.name, file.size, source, None);
                        store.start_upload(id, cx);
                        // A video goes with still frames of it.
                        if video {
                            store.attach_stills(id, file.path, name, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Attaches a pasted picture, made ready off the UI thread (see [`Pasted`]); Send waits
    /// meanwhile.
    pub(crate) fn attach_picture(&mut self, picture: Image, cx: &mut Context<'_, Self>) {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        self.attachments.begin_adding();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let pasted = cx
                .background_spawn(async move { Pasted::new(picture, seconds) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.attachments.added();
                let (size, source) = (pasted.size(), UploadSource::Bytes(pasted.bytes()));
                let id = store
                    .attachments
                    .add(pasted.name, size, source, Some(pasted.picture));
                store.start_upload(id, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Tries a failed upload again (not one that never can work).
    pub(crate) fn retry_attachment(&mut self, id: AttachmentId, cx: &mut Context<'_, Self>) {
        if self.attachments.retry(id) {
            self.start_upload(id, cx);
            cx.notify();
        }
    }

    /// Takes an attachment out of the composer, stopping its upload.
    pub(crate) fn remove_attachment(&mut self, id: AttachmentId, cx: &mut Context<'_, Self>) {
        if self.attachments.remove(id) {
            cx.notify();
        }
    }

    /// Uploads `id` on a background thread; its progress comes back each whole percent, and
    /// it stops once the attachment is removed.
    pub(super) fn start_upload(&mut self, id: AttachmentId, cx: &mut Context<'_, Self>) {
        let Some(item) = self.attachments.get(id) else {
            return;
        };
        let (name, source, size) = (item.name.clone(), item.source.clone(), item.size);
        let (stop, removed) = (item.stop_flag(), item.stop_flag());
        let backend = Arc::clone(&self.backend);
        let (progress, sent) = smol::channel::unbounded::<u64>();
        let upload = cx.background_spawn(async move {
            let mut told = None;
            backend.upload(&name, &source, &mut |bytes| {
                if stop.is_set() {
                    return ControlFlow::Break(());
                }
                let percent = bytes.saturating_mul(100) / size.max(1);
                if told != Some(percent) {
                    told = Some(percent);
                    if progress.try_send(bytes).is_err() {
                        log::debug!("nobody follows the upload of {name} any more");
                    }
                }
                ControlFlow::Continue(())
            })
        });
        cx.spawn(async move |this, cx| {
            while let Ok(bytes) = sent.recv().await {
                update_or_log(&this, cx, |store, cx| {
                    if store.attachments.progress(id, bytes) {
                        cx.notify();
                    }
                });
            }
            let outcome = match upload.await {
                Ok(file) => Ok(file),
                Err(error) if removed.is_set() => {
                    log::debug!("the upload of attachment {id} stopped, removed: {error}");
                    return;
                }
                Err(error) => {
                    log::warn!("couldn't upload attachment {id}: {error}");
                    Err(failure(&error))
                }
            };
            update_or_log(&this, cx, |store, cx| {
                if store.attachments.finish(id, outcome) {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// Why an upload failed, for its chip: the server's words, or what went wrong here.
fn failure(error: &Error) -> String {
    match error {
        Error::Local(cause) => capitalized(&cause.to_string()),
        other => other
            .server_message()
            .map_or_else(|| capitalized(&other.to_string()), capitalized),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use emberhut_api::chat::ChatFile;
    use emberhut_api::ids::RoomId;
    use emberhut_api::talk::TalkRequest;
    use gpui_kit::{Entity, ImageFormat, TestAppContext};

    use super::super::ThemeState;
    use super::super::attachments::Stage;
    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
        let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
    }

    fn stages(store: &Entity<Store>, cx: &mut TestAppContext) -> Vec<Stage> {
        store.read_with(cx, |store, _| {
            store
                .attachments()
                .items()
                .iter()
                .map(|item| item.stage.clone())
                .collect()
        })
    }

    #[gpui_kit::test]
    fn two_dropped_files_upload_and_go_with_one_message(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.pdf"), dir.path().join("b.png"));
        std::fs::write(&a, b"pdf").unwrap();
        std::fs::write(&b, b"png!").unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);

        store.update(cx, |store, cx| {
            store.attach_paths(&[a, b, dir.path().to_path_buf()], cx);
            assert!(!store.talk("Look", cx), "send waits for the files");
        });
        cx.run_until_parked();

        let mut uploads: Vec<Call> = backend.calls();
        uploads.sort_by_key(|call| format!("{call:?}"));
        assert_eq!(
            uploads,
            [Call::Upload("a.pdf".into()), Call::Upload("b.png".into())]
        );
        let stored = [FakeBackend::stored("a.pdf"), FakeBackend::stored("b.png")];
        assert_eq!(
            stages(&store, cx),
            stored.clone().map(Stage::Ready).to_vec(),
            "the folder is left out"
        );
        store.update(cx, |store, cx| assert!(store.talk("Look", cx)));
        cx.run_until_parked();

        let talk = TalkRequest::text(RoomId::MAIN, "Look").with_files(stored.to_vec());
        assert_eq!(backend.calls().last(), Some(&Call::Talk(talk)));
        store.read_with(cx, |store, _| {
            assert!(store.attachments().items().is_empty());
            assert_eq!(store.attachments().sent_size("/uploads/b.png"), Some(4));
            let pending = store.pending().map(|pending| pending.files.len());
            assert_eq!(pending, Some(2), "the pending message shows its files");
        });
    }

    #[gpui_kit::test]
    fn a_failed_upload_says_why_and_can_be_tried_again(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"notes").unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_upload(Err(Error::Unreachable("connection refused".into())));
        let store = store(cx, &backend);

        store.update(cx, |store, cx| store.attach_paths(&[path], cx));
        cx.run_until_parked();
        assert_eq!(
            stages(&store, cx),
            [Stage::Failed {
                reason: "Can't reach the server".into(),
                retryable: true
            }]
        );
        store.update(cx, |store, cx| {
            assert!(!store.talk("", cx), "nothing to send");
            let id = store.attachments().items()[0].id;
            store.retry_attachment(id, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            stages(&store, cx),
            [Stage::Ready(FakeBackend::stored("notes.txt"))]
        );
        assert_eq!(
            backend.calls(),
            [
                Call::Upload("notes.txt".into()),
                Call::Upload("notes.txt".into())
            ]
        );
    }

    #[gpui_kit::test]
    fn a_pasted_picture_uploads_from_memory(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        let picture = Image::from_bytes(ImageFormat::Png, b"\x89PNG".to_vec());

        store.update(cx, |store, cx| store.attach_picture(picture, cx));
        cx.run_until_parked();

        let calls: [Call; 1] = backend.calls().try_into().unwrap();
        let [Call::Upload(name)] = calls else {
            panic!("one upload, not {calls:?}")
        };
        assert!(
            name.starts_with("pasted-") && Path::new(&name).extension() == Some("png".as_ref()),
            "{name}"
        );
        store.read_with(cx, |store, _| {
            let item = &store.attachments().items()[0];
            assert!(item.picture.is_some(), "its thumbnail");
            assert_eq!(item.size, 4);
            assert!(matches!(item.stage, Stage::Ready(ChatFile { .. })));
        });
    }

    #[gpui_kit::test]
    fn removing_an_attachment_stops_its_upload(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"pdf").unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        // Removed as soon as it shows, before its upload has sent anything.
        let _removing = cx.update(|cx| {
            cx.observe(&store, |store, cx| {
                let first = store
                    .read(cx)
                    .attachments()
                    .items()
                    .first()
                    .map(|item| item.id);
                if let Some(id) = first {
                    store.update(cx, |store, cx| store.remove_attachment(id, cx));
                }
            })
        });
        store.update(cx, |store, cx| store.attach_paths(&[path], cx));
        cx.run_until_parked();
        assert_eq!(stages(&store, cx), []);
        assert_eq!(
            backend.calls(),
            [
                Call::Upload("a.pdf".into()),
                Call::UploadStopped("a.pdf".into())
            ]
        );
        store.update(cx, |store, cx| assert!(!store.talk("", cx)));
    }

    #[gpui_kit::test]
    fn a_file_over_the_limit_is_never_sent_nor_retried(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.mov");
        // Sparse: it takes no room on disk.
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_SIZE + 1)
            .unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| store.attach_paths(&[path], cx));
        cx.run_until_parked();
        store.update(cx, |store, cx| {
            let id = store.attachments().items()[0].id;
            store.retry_attachment(id, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            stages(&store, cx),
            [Stage::Failed {
                reason: TOO_LARGE.into(),
                retryable: false
            }]
        );
        assert_eq!(backend.calls(), []);
    }

    #[gpui_kit::test]
    fn a_pasted_tiff_goes_as_png(cx: &mut TestAppContext) {
        let pixels = image::RgbImage::from_raw(1, 1, vec![9, 9, 9]).unwrap();
        let mut tiff = Vec::new();
        pixels
            .write_to(
                &mut std::io::Cursor::new(&mut tiff),
                image::ImageFormat::Tiff,
            )
            .unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        let picture = Image::from_bytes(ImageFormat::Tiff, tiff);
        store.update(cx, |store, cx| store.attach_picture(picture, cx));
        cx.run_until_parked();
        let calls: [Call; 1] = backend.calls().try_into().unwrap();
        let [Call::Upload(name)] = calls else {
            panic!("one upload, not {calls:?}")
        };
        assert_eq!(Path::new(&name).extension(), Some("png".as_ref()), "{name}");
    }

    #[test]
    fn the_too_large_reason_names_the_limit() {
        let limit = format!("{} MiB", MAX_SIZE >> 20);
        assert!(TOO_LARGE.contains(&limit), "{TOO_LARGE} / {limit}");
    }

    #[test]
    fn a_failure_reads_as_the_servers_words_or_ours() {
        let refused = Error::Refused {
            status: Some(400),
            message: Some("file is larger than 500 MB".into()),
        };
        assert_eq!(failure(&refused), "File is larger than 500 MB");
        let unreachable = Error::Unreachable("connection refused".into());
        assert_eq!(failure(&unreachable), "Can't reach the server");
        let unread = Error::Local("couldn't read the file to upload".into());
        assert_eq!(failure(&unread), "Couldn't read the file to upload");
    }
}
