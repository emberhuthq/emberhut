//! Fetching a server file into the app's folder of kept files (`kept`): streamed with this
//! device's session, once however many ask for it at the same time, and at once when it is kept
//! already (a reopen is instant, across launches too). Files left untouched for a day are
//! cleared out when the app starts.
//!
//! A download that runs past [`BUSY_AFTER`] shows on the status line with how far it is, and
//! one that fails says so there; the file's chip says the same while it lasts ([`Opening`]).

#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use gpui_kit::AppContext as _;
use gpui_kit::{App, Context, SharedString, Task};
use smol::channel::{Receiver, Sender};

use super::RemoteFile;
use super::status::{BUSY_AFTER, FileStatus, Note, SHOWN_FOR, percent};
use crate::config::ServerUrl;
use crate::store::fetched::Failure;
use crate::store::kept::{fetch, kept_path, sweep};
use crate::store::{Store, update_or_log};

/// Where fetching a file stands on its chip, while it isn't here yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Opening {
    /// Coming down: `done` bytes of `total` (when the server gives it).
    Fetching {
        /// The bytes here so far.
        done: u64,
        /// The file's size.
        total: Option<u64>,
    },
    /// It couldn't be fetched or kept, with the reason worded for the user.
    Failed(String),
}

impl Opening {
    /// What its chip says: `Downloading… 42%`, or why it failed.
    #[must_use]
    pub(crate) fn note(&self) -> String {
        match self {
            Self::Fetching { done, total } => match percent(*done, *total) {
                Some(percent) => format!("Downloading… {percent}%"),
                None => "Downloading…".to_owned(),
            },
            Self::Failed(reason) => reason.clone(),
        }
    }
}

/// Hands a kept file to the system: to its default app, or to the file manager.
type Opener = Rc<dyn Fn(&Path, &mut App)>;

/// The answer to one ask for a file: where it is kept, or why it couldn't be.
type Answer = Result<PathBuf, String>;

/// The files kept, the ones coming down and who waits for each, and the status line.
pub(crate) struct KeptFiles {
    /// The app's folder of fetched files (none when the system has no cache folder).
    root: Option<PathBuf>,
    /// What each file's chip says while it comes down or after it failed, by server address.
    openings: HashMap<String, Opening>,
    /// Who waits for each file coming down, by server address.
    waiting: HashMap<String, Waiting>,
    /// The status line.
    pub(super) status: FileStatus,
    /// The reports' titles read for their cards in the chat, by server address (`None`: being
    /// read, or the page has none).
    pub(super) titles: HashMap<String, Option<SharedString>>,
    /// What opens a kept file (the system; a recorder in tests).
    pub(super) open: Opener,
    /// What shows a kept file in its folder (the system's file manager; a recorder in tests).
    pub(super) reveal: Opener,
}

impl fmt::Debug for KeptFiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeptFiles")
            .field("root", &self.root)
            .field("openings", &self.openings)
            .field("waiting", &self.waiting.len())
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl KeptFiles {
    /// In the platform's cache directory, named like the config folder: the bundle id on
    /// macOS, `emberhut` elsewhere.
    #[must_use]
    pub(crate) fn in_cache() -> Self {
        Self::in_folder(crate::config::cache_dir().map(|app| app.join("files")))
    }

    /// Kept files in `root`.
    #[must_use]
    pub(crate) fn in_folder(root: Option<PathBuf>) -> Self {
        Self {
            root,
            openings: HashMap::new(),
            waiting: HashMap::new(),
            status: FileStatus::default(),
            titles: HashMap::new(),
            open: Rc::new(|path, cx| cx.open_with_system(path)),
            reveal: Rc::new(|path, cx| cx.reveal_path(path)),
        }
    }

    /// Where `file`, on `server`, is kept once it is here; `None` without a cache folder. The
    /// folder is named after the file's whole address, so two servers' files of the same path
    /// never share one.
    #[must_use]
    pub(crate) fn path_of(&self, server: &ServerUrl, file: &RemoteFile) -> Option<PathBuf> {
        Some(kept_path(
            self.root.as_deref()?,
            &server.join(&file.path),
            &file.name,
        ))
    }
}

/// Who waits for a file coming down.
#[derive(Debug, Default)]
struct Waiting {
    /// Where each ask hears the answer.
    told: Vec<Sender<Answer>>,
    /// Whether one of them was the user's (a fetch only the app asked for, a picture in the
    /// chat, shows nowhere).
    shown: bool,
}

/// Whether a fetch shows on the status line and the file's chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    /// The user asked: how far it is and whether it failed show.
    Shown,
    /// The app asked (a picture to show, a report's title): nothing shows.
    Quiet,
}

/// One ask for a file: wait on it for where it is kept.
#[derive(Debug)]
pub(crate) struct Fetching(Receiver<Answer>);

impl Fetching {
    /// An ask answered already.
    fn answered(answer: Answer) -> Self {
        let (tell, told) = smol::channel::bounded(1);
        if tell.try_send(answer).is_err() {
            log::debug!("a file's answer had no one to go to");
        }
        Self(told)
    }

    /// Where the file is kept once it is here, or why it couldn't be (the status line said so).
    ///
    /// # Errors
    ///
    /// The reason, worded for the user.
    pub(crate) async fn kept(self) -> Answer {
        self.0
            .recv()
            .await
            .unwrap_or_else(|_| Err("The download stopped".to_owned()))
    }
}

/// Sets a flag when it is dropped: the download it stands for stops at its next chunk.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Store {
    /// Where fetching `url` stands on its chip, while it isn't here yet or after it failed.
    #[must_use]
    pub(crate) fn opening(&self, url: &str) -> Option<&Opening> {
        self.kept.openings.get(url)
    }

    /// Where `file` is kept, if it is here whole now (a fetch moves it into place only once it
    /// is all here).
    #[must_use]
    pub(crate) fn kept_file(&self, file: &RemoteFile) -> Option<PathBuf> {
        self.kept
            .path_of(self.server(), file)
            .filter(|path| path.is_file())
    }

    /// What the status line says about files, if anything.
    #[must_use]
    pub(crate) fn file_status(&self) -> Option<&Note> {
        self.kept.status.note()
    }

    /// Clears the kept files no one used for a day, off the UI thread.
    pub(in crate::store) fn sweep_kept_files(&self, cx: &mut Context<'_, Self>) {
        let Some(root) = self.kept.root.clone() else {
            return;
        };
        let now = SystemTime::now();
        cx.background_spawn(async move { sweep(&root, now) })
            .detach();
    }

    /// The whole of `file`, here on disk, for the user: kept already, on its way for someone
    /// else, or fetched now, its download showing. Wait on the answer for where it is.
    pub(crate) fn fetch_file(&mut self, file: &RemoteFile, cx: &mut Context<'_, Self>) -> Fetching {
        self.fetch(file, Shown::Shown, cx)
    }

    /// The whole of `file`, here on disk, as [`Store::fetch_file`], `shown` saying whether its
    /// download shows.
    pub(super) fn fetch(
        &mut self,
        file: &RemoteFile,
        shown: Shown,
        cx: &mut Context<'_, Self>,
    ) -> Fetching {
        let shown = shown == Shown::Shown;
        if !ServerUrl::is_path(&file.path) {
            log::warn!(
                "not fetching a file that isn't on the server: {}",
                file.path
            );
            return Fetching::answered(Err("It isn't on the server".to_owned()));
        }
        let Some(path) = self.kept.path_of(self.server(), file) else {
            let reason = "This device has no cache folder".to_owned();
            if shown {
                self.kept
                    .openings
                    .insert(file.path.clone(), Opening::Failed(reason.clone()));
                cx.notify();
            }
            return Fetching::answered(Err(reason));
        };
        let (tell, told) = smol::channel::bounded(1);
        if let Some(waiting) = self.kept.waiting.get_mut(&file.path) {
            waiting.told.push(tell);
            waiting.shown |= shown;
            return Fetching(told);
        }
        let waiting = Waiting {
            told: vec![tell],
            shown,
        };
        self.kept.waiting.insert(file.path.clone(), waiting);
        // A failure from before is tried afresh.
        if shown {
            self.kept.openings.remove(&file.path);
        }
        self.start_fetch(file.clone(), path, cx);
        Fetching(told)
    }

    /// Streams `file` into `path` off the UI thread, telling its chip and the status line how
    /// far it is, then everyone waiting for it.
    fn start_fetch(&mut self, file: RemoteFile, path: PathBuf, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = StopOnDrop(Arc::clone(&stopped));
        let (told, got) = smol::channel::unbounded::<(u64, Option<u64>)>();
        let (url, into) = (file.path.clone(), path.clone());
        let fetched = cx.background_spawn(async move {
            let mut last: Option<Option<u8>> = None;
            fetch(&*backend, &url, &into, &mut |done, total| {
                if stopped.load(Ordering::Relaxed) {
                    return ControlFlow::Break(());
                }
                let now = percent(done, total);
                if last != Some(now) || total.is_none() {
                    last = Some(now);
                    if told.try_send((done, total)).is_err() {
                        log::debug!("nobody follows the download of {url} any more");
                    }
                }
                ControlFlow::Continue(())
            })
        });
        // Past a moment, the status line says it is downloading.
        let busy = file.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BUSY_AFTER).await;
            update_or_log(&this, cx, |store, cx| {
                if !store.kept.waiting.get(&busy.path).is_some_and(|w| w.shown) {
                    return;
                }
                let percent = match store.kept.openings.get(&busy.path) {
                    Some(Opening::Fetching { done, total }) => percent(*done, *total),
                    Some(Opening::Failed(_)) | None => None,
                };
                store.kept.status.busy(&busy.path, &busy.name, percent);
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            // Dropped with this task (the app closing): the download stops.
            let _stop = stop;
            while let Ok((done, total)) = got.recv().await {
                update_or_log(&this, cx, |store, cx| {
                    if !store.kept.waiting.get(&file.path).is_some_and(|w| w.shown) {
                        return;
                    }
                    let fetching = Opening::Fetching { done, total };
                    store.kept.openings.insert(file.path.clone(), fetching);
                    store.kept.status.progress(&file.path, percent(done, total));
                    cx.notify();
                });
            }
            let fetched = fetched.await;
            update_or_log(&this, cx, |store, cx| {
                store.fetch_ended(&file, &fetched.map(|()| path), cx);
            });
        })
        .detach();
    }

    /// `file`'s fetch came back: everyone waiting hears, its chip and the status line say how
    /// it went.
    fn fetch_ended(&mut self, file: &RemoteFile, answer: &Answer, cx: &mut Context<'_, Self>) {
        let waiting = self.kept.waiting.remove(&file.path).unwrap_or_default();
        self.kept.status.ended(&file.path);
        match answer {
            Ok(_) => {
                self.kept.openings.remove(&file.path);
            }
            Err(reason) if waiting.shown => {
                self.kept
                    .openings
                    .insert(file.path.clone(), Opening::Failed(reason.clone()));
                self.show_file_note(format!("Couldn't download {}", file.name), true, cx);
            }
            Err(_) => {}
        }
        for waiting in waiting.told {
            if waiting.try_send(answer.clone()).is_err() {
                log::debug!("one waiting for {} went away", file.path);
            }
        }
        cx.notify();
    }

    /// The bytes of the server's file at `path`, for the app to show (a picture, a report):
    /// through the kept files, quietly, so a reopen (across launches too) needs no download;
    /// straight from the server into memory where there is no cache folder.
    pub(crate) fn file_bytes(
        &mut self,
        path: &str,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<Vec<u8>, Failure>> {
        if self.kept.root.is_none() {
            let (backend, path) = (Arc::clone(&self.backend), path.to_owned());
            return cx.background_spawn(async move {
                backend
                    .file(&path)
                    .map_err(|error| Failure::passing(error.to_string()))
            });
        }
        let name = path
            .split(['?', '#'])
            .next()
            .and_then(|path| path.rsplit('/').next())
            .filter(|name| !name.is_empty())
            .unwrap_or("file");
        let fetching = self.fetch(&RemoteFile::new(path, name), Shown::Quiet, cx);
        cx.background_spawn(async move {
            let kept = fetching.kept().await.map_err(Failure::passing)?;
            std::fs::read(&kept).map_err(|error| Failure::passing(error.to_string()))
        })
    }

    /// Shows `text` on the status line for a moment.
    pub(super) fn show_file_note(
        &mut self,
        text: String,
        failed: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let number = self.kept.status.show(text, failed);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOWN_FOR).await;
            update_or_log(&this, cx, |store, cx| {
                if store.kept.status.expire(number) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Keeps fetched files in `root`, and notes each one opened in `opened` (tests); one
    /// shown in its folder instead is noted too, after `shown in folder: `.
    #[cfg(test)]
    pub(crate) fn keep_files_in(&mut self, root: &Path, opened: &Rc<RefCell<Vec<PathBuf>>>) {
        let (opened, revealed) = (Rc::clone(opened), Rc::clone(opened));
        self.kept = KeptFiles {
            open: Rc::new(move |path, _| opened.borrow_mut().push(path.to_path_buf())),
            reveal: Rc::new(move |path, _| {
                let noted = format!("shown in folder: {}", path.display());
                revealed.borrow_mut().push(PathBuf::from(noted));
            }),
            ..KeptFiles::in_folder(Some(root.to_path_buf()))
        };
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Entity, TestAppContext};

    use super::*;
    use crate::backend::Backend;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::store::files::Use;

    type Opened = Rc<RefCell<Vec<PathBuf>>>;

    fn store(
        cx: &mut TestAppContext,
        backend: &Arc<FakeBackend>,
        root: &Path,
    ) -> (Entity<Store>, Opened) {
        let backend = Arc::clone(backend) as Arc<dyn Backend>;
        let opened = Opened::default();
        let store = cx.new(|_| {
            let mut store = Store::for_tests(backend);
            store.keep_files_in(root, &opened);
            store
        });
        (store, opened)
    }

    #[gpui_kit::test]
    fn a_file_is_fetched_once_kept_privately_then_opened_from_here(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_download(Ok(b"the report".to_vec()));
        let (store, opened) = store(cx, &backend, root.path());
        let url = "/uploads/1a2b-report.pdf";

        store.update(cx, |store, cx| {
            store.use_file(RemoteFile::new(url, "report.pdf"), Use::OpenInApp, cx);
        });
        cx.run_until_parked();

        store.read_with(cx, |store, _| assert_eq!(store.opening(url), None));
        let at = store.read_with(cx, |store, _| store.server().join(url));
        let kept = kept_path(root.path(), &at, "report.pdf");
        assert_eq!(
            *opened.borrow(),
            std::slice::from_ref(&kept),
            "opened once kept"
        );
        assert_eq!(std::fs::read(&kept).unwrap(), b"the report");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&kept), 0o600);
            assert_eq!(mode(kept.parent().unwrap()), 0o700);
        }
        store.update(cx, |store, cx| {
            store.use_file(RemoteFile::new(url, "report.pdf"), Use::OpenInApp, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [Call::Download(url.into())],
            "the second open uses the kept file"
        );
        assert_eq!(*opened.borrow(), [kept.clone(), kept]);
    }

    #[gpui_kit::test]
    fn two_asks_at_once_download_once(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_download(Ok(b"bytes".to_vec()));
        let (store, _) = store(cx, &backend, root.path());
        let file = RemoteFile::new("/uploads/a.pdf", "a.pdf");
        let (first, second) = store.update(cx, |store, cx| {
            (store.fetch_file(&file, cx), store.fetch_file(&file, cx))
        });
        cx.run_until_parked();
        let at = store.read_with(cx, |store, _| store.server().join(&file.path));
        let kept = kept_path(root.path(), &at, &file.name);
        // Both were answered when the download ended.
        let (first, second) = smol::block_on(async { (first.kept().await, second.kept().await) });
        assert_eq!(first, Ok(kept.clone()));
        assert_eq!(second, Ok(kept));
        assert_eq!(backend.calls(), [Call::Download(file.path.clone())]);
    }

    #[gpui_kit::test]
    fn a_file_that_cant_be_fetched_says_so_and_leaves_nothing(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        let (store, opened) = store(cx, &backend, root.path());
        let url = "/uploads/a.pdf";
        store.update(cx, |store, cx| {
            store.use_file(RemoteFile::new(url, "a.pdf"), Use::OpenInApp, cx);
            store.use_file(
                RemoteFile::new("https://example.com/a.pdf", "a.pdf"),
                Use::OpenInApp,
                cx,
            );
        });
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.opening(url),
                Some(&Opening::Failed("Couldn't fetch it from the server".into()))
            );
            assert_eq!(store.opening("https://example.com/a.pdf"), None);
            assert_eq!(
                store.file_status().map(Note::text).as_deref(),
                Some("Couldn't download a.pdf")
            );
        });
        let at = store.read_with(cx, |store, _| store.server().join(url));
        let folder = kept_path(root.path(), &at, "a.pdf");
        let left: Vec<_> = std::fs::read_dir(folder.parent().unwrap())
            .unwrap()
            .collect();
        assert!(left.is_empty(), "no partial stays: {left:?}");
        assert!(opened.borrow().is_empty());
        assert_eq!(backend.calls(), [Call::Download(url.into())]);
    }

    #[gpui_kit::test]
    fn a_file_that_could_run_is_shown_in_its_folder_never_opened(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        for _ in 0..4 {
            backend.push_download(Ok(b"#!/bin/sh".to_vec()));
        }
        let (store, opened) = store(cx, &backend, root.path());
        store.update(cx, |store, cx| {
            for (path, name) in [
                ("/uploads/x.command", "the invoice"),
                ("/uploads/y.pdf", "y.sh"),
                ("/uploads/z.exe", "z.exe"),
                ("/uploads/w.terminal", "w.terminal"),
            ] {
                store.use_file(RemoteFile::new(path, name), Use::OpenInApp, cx);
            }
        });
        cx.run_until_parked();
        let noted: Vec<String> = opened
            .borrow()
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        assert_eq!(noted.len(), 4, "{noted:?}");
        assert!(
            noted
                .iter()
                .all(|note| note.starts_with("shown in folder: ")),
            "the opener is never called: {noted:?}"
        );
        store.read_with(cx, |store, _| {
            assert!(
                store
                    .file_status()
                    .is_some_and(|note| note.text().contains("runs as a program")),
                "{:?}",
                store.file_status()
            );
        });
    }

    #[test]
    fn two_servers_files_of_one_path_are_kept_apart() {
        let kept = KeptFiles::in_folder(Some(PathBuf::from("/cache/files")));
        let file = RemoteFile::new("/api/artifacts/66", "plan.pdf");
        let at = |server: &str| kept.path_of(&ServerUrl::parse(server).unwrap(), &file);
        assert_ne!(at("https://one.example"), at("https://two.example"));
        assert_eq!(at("https://one.example"), at("https://one.example"));
        assert_eq!(
            KeptFiles::in_folder(None)
                .path_of(&ServerUrl::parse("https://one.example").unwrap(), &file),
            None
        );
    }

    #[test]
    fn the_chip_says_how_far_it_is() {
        let at = |done, total| Opening::Fetching { done, total }.note();
        assert_eq!(at(21, Some(50)), "Downloading… 42%");
        assert_eq!(at(9, None), "Downloading…");
        assert_eq!(at(0, Some(0)), "Downloading…");
        assert_eq!(Opening::Failed("Gone".into()).note(), "Gone");
    }
}
