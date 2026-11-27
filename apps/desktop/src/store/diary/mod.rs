//! The diary and the quick note: the user's notes by day, kept on this device in `diary.json`
//! with the changes the server hasn't taken yet, so a note is saved at once and works offline.
//! Changes are sent one by one, each by its uid, when the server can be reached (at once after
//! a save, then every 30 s while some wait); then the diary is read again as the server has it.
//! Opening the diary or a quick note reads it too.
//!
//! What the diary holds and how it words its days are pure ([`book`], [`words`]), tested on
//! their own.

pub(crate) mod book;
pub(crate) mod words;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_api::ids::NoteUid;
use emberhut_api::notes::NoteSave;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use self::book::{Book, NewNote};
use super::gate::reason;
use super::poll::{self, Kicked, Kicker};
use super::{Store, update_or_log};
use crate::backend::Error;
use crate::config::AppFiles;

/// The file's name in the app's directory.
const FILE: &str = "diary.json";
/// How long to wait before sending again what the server couldn't take.
const RETRY: Duration = Duration::from_secs(30);

/// The diary's data in the store.
#[derive(Debug)]
pub(crate) struct DiaryData {
    book: Book,
    zone: TimeZone,
    /// Wakes the sending loop; its end waits here until the loop starts.
    kicker: Kicker,
    kicked: Option<Kicked>,
    /// The number of the last save of the file asked for, and of the last one written.
    saves: u64,
    written: Arc<Mutex<u64>>,
    /// Why the server refused the last note it refused, until the diary is closed or the note
    /// saved again.
    problem: Option<String>,
}

impl DiaryData {
    /// The diary, as read from `files` at launch (empty when there is none or it can't be read).
    pub(super) fn load(files: Option<&AppFiles>) -> Self {
        let book = files.map(path).map_or_default(|path| read(&path));
        let (kicker, waiting) = poll::kicker();
        Self {
            book,
            zone: words::zone(),
            kicker,
            kicked: Some(waiting),
            saves: 0,
            written: Arc::default(),
            problem: None,
        }
    }

    /// What the diary holds.
    #[must_use]
    pub(crate) fn book(&self) -> &Book {
        &self.book
    }

    /// The zone its days are in.
    #[must_use]
    pub(crate) fn zone(&self) -> &TimeZone {
        &self.zone
    }

    /// Why the server refused a note just now, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

fn path(files: &AppFiles) -> PathBuf {
    files.dir().join(FILE)
}

/// The book kept at `path`; an empty one when there is none or it can't be read.
fn read(path: &std::path::Path) -> Book {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .inspect_err(|error| log::warn!("{} can't be read: {error}", path.display()))
            .unwrap_or_default(),
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("can't read {}: {error}", path.display());
            }
            Book::default()
        }
    }
}

/// Writes `book` through a temporary file and a rename, so a reader never sees half of it.
fn write(path: &std::path::Path, book: &Book) {
    let temporary = path.with_extension("json.tmp");
    let outcome = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| serde_json::to_vec(book).map_err(std::io::Error::from))
        .and_then(|bytes| std::fs::write(&temporary, bytes))
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = outcome {
        log::warn!("can't keep the diary in {}: {error}", path.display());
    }
}

/// The digits of a uid's hex.
const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// How many uids were made without randomness, so two in the same instant differ.
static FALLBACK_UIDS: AtomicU64 = AtomicU64::new(0);

/// A new note's uid: 16 random bytes as a version 4 UUID. Without the system's randomness (it
/// doesn't fail on the platforms the app runs on), the time and a counter, unique on this device.
fn new_uid() -> NoteUid {
    let mut bytes = [0_u8; 16];
    if let Err(error) = getrandom::getrandom(&mut bytes) {
        log::warn!("no randomness for a note's uid ({error}): it is made from the time");
        let count = FALLBACK_UIDS.fetch_add(1, Ordering::Relaxed);
        return NoteUid::new(format!("t{:x}-{count}", Timestamp::now().as_nanosecond()));
    }
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
    }
    NoteUid::new(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// What became of a note sent to the server.
#[derive(Debug)]
enum Outcome {
    /// It took it, keeping this copy (none for a deletion).
    Taken(Option<emberhut_api::notes::Note>),
    /// It refused it for good (a 4xx: it never will take that note), with this answer.
    Refused(Error),
    /// It couldn't be asked, or may take it later: it is tried again.
    Later,
}

impl Outcome {
    /// What `sent`, the server's answer to a note, means for the note.
    fn of(sent: Result<emberhut_api::notes::NoteSaved, Error>) -> Self {
        match sent {
            Ok(saved) => Self::Taken(saved.note),
            // A timeout and too many requests are worth trying again; any other 4xx isn't.
            Err(
                error @ Error::Refused {
                    status: Some(status @ 400..=499),
                    ..
                },
            ) if status != 408 && status != 429 => Self::Refused(error),
            Err(_) => Self::Later,
        }
    }
}

impl Store {
    /// The diary.
    #[must_use]
    pub(crate) fn diary(&self) -> &DiaryData {
        &self.diary
    }

    /// Saves a quick note (`editing` none) or the changes to one (empty text deletes it): on
    /// this device at once, then to the server. Whether anything changed.
    pub(crate) fn save_note(
        &mut self,
        text: &str,
        editing: Option<&NoteUid>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let new = NewNote {
            uid: new_uid(),
            now: Timestamp::now(),
            zone: &self.diary.zone,
        };
        if !self.diary.book.save(text, editing, new) {
            return false;
        }
        self.diary.problem = None;
        self.keep_diary(cx);
        self.diary.kicker.kick();
        cx.notify();
        true
    }

    /// Deletes the note `uid`.
    pub(crate) fn delete_note(&mut self, uid: &NoteUid, cx: &mut Context<'_, Self>) {
        self.save_note("", Some(uid), cx);
    }

    /// Keeps what the quick note had when it closed unsaved, for the next one.
    pub(crate) fn keep_note_draft(&mut self, draft: String, cx: &mut Context<'_, Self>) {
        if self.diary.book.set_draft(draft) {
            self.keep_diary(cx);
        }
    }

    /// Sends what waits and reads the diary again (the diary or a quick note opened).
    pub(crate) fn sync_diary(&self) {
        self.diary.kicker.kick();
    }

    /// Writes the diary off the UI thread. Each save is numbered: one that runs after a newer
    /// one is dropped, so the newest is what stays on disk.
    fn keep_diary(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = self.files.as_ref().map(path) else {
            return;
        };
        self.diary.saves += 1;
        let (number, book, written) = (
            self.diary.saves,
            self.diary.book.clone(),
            Arc::clone(&self.diary.written),
        );
        cx.background_spawn(async move {
            let mut last = written.lock().unwrap_or_else(PoisonError::into_inner);
            if *last < number {
                write(&path, &book);
                *last = number;
            }
        })
        .detach();
    }

    /// Starts the sending loop (once); what waits from the last launch goes at once.
    pub(crate) fn start_diary(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.diary.kicked.take() {
            if self.diary.book.has_unsent() {
                self.diary.kicker.kick();
            }
            run(kicked, cx);
        }
    }
}

/// Runs the sending loop: on each kick (or every [`RETRY`] while changes wait), the changes go
/// out one by one, then the diary is read again. Once the server is out of reach, the rest wait.
fn run(kicked: Kicked, cx: &mut Context<'_, Store>) {
    cx.spawn(async move |this, cx| {
        loop {
            let Some(waiting) = update_or_log(&this, cx, |store, _| store.diary.book.has_unsent())
            else {
                return;
            };
            if waiting {
                let timer = cx.background_executor().timer(RETRY);
                smol::future::or(
                    async {
                        timer.await;
                    },
                    kicked.wait(),
                )
                .await;
            } else {
                kicked.wait().await;
            }
            let Some((backend, sends)) = update_or_log(&this, cx, |store, _| {
                (Arc::clone(&store.backend), store.diary.book.to_send())
            }) else {
                return;
            };
            let mut reached = true;
            for send in sends {
                let one = Arc::clone(&backend);
                let note: NoteSave = send.clone();
                let sent = cx
                    .background_spawn(async move { one.save_note(&note) })
                    .await;
                if let Err(error) = &sent {
                    log::info!("a note wasn't sent: {error}");
                }
                let outcome = Outcome::of(sent);
                if matches!(outcome, Outcome::Later) {
                    reached = false;
                    break;
                }
                update_or_log(&this, cx, |store, cx| {
                    let left = match outcome {
                        Outcome::Taken(kept) => store.diary.book.sent(&send, kept),
                        Outcome::Refused(error) => {
                            // Kept with its reason: the diary shows it, marked, and says why,
                            // as the main window does.
                            let why = reason(&error);
                            store.diary.problem = Some(format!("A note wasn't saved: {why}"));
                            store.action_failed("A note wasn't saved", &error, cx);
                            store.diary.book.refused(&send, why)
                        }
                        Outcome::Later => false,
                    };
                    if left {
                        store.keep_diary(cx);
                        cx.notify();
                    }
                });
            }
            if !reached {
                continue;
            }
            let notes = cx.background_spawn(async move { backend.notes() }).await;
            match notes {
                Ok(page) => {
                    update_or_log(&this, cx, |store, cx| {
                        if store.diary.book.take_notes(page.notes) {
                            store.keep_diary(cx);
                            cx.notify();
                        }
                    });
                }
                Err(error) => log::info!("the diary wasn't read: {error}"),
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::notes::NotesPage;
    use gpui_kit::TestAppContext;
    use serde_json::json;

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    #[test]
    fn a_new_uid_is_a_random_uuid_the_server_takes() {
        let (one, two) = (new_uid(), new_uid());
        assert_ne!(one, two);
        assert_eq!(one.as_str().len(), 36);
        assert_eq!(one.as_str().as_bytes()[14], b'4', "version 4");
        assert!(
            one.as_str()
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-')
        );
    }

    #[test]
    fn a_note_refused_for_good_is_kept_with_why_and_one_unsent_tried_again() {
        let refused = |status| {
            Outcome::of(Err(Error::Refused {
                status: Some(status),
                message: None,
            }))
        };
        assert!(matches!(
            Outcome::of(Ok(emberhut_api::notes::NoteSaved {
                ok: true,
                note: None
            })),
            Outcome::Taken(None)
        ));
        for refusal in [400, 404, 413, 422] {
            assert!(matches!(refused(refusal), Outcome::Refused(_)), "{refusal}");
        }
        for later in [408, 429, 500, 503] {
            assert!(matches!(refused(later), Outcome::Later), "{later}");
        }
        assert!(matches!(
            Outcome::of(Err(Error::Unreachable("down".into()))),
            Outcome::Later
        ));
        assert!(matches!(
            Outcome::of(Err(Error::SignInRequired)),
            Outcome::Later
        ));
    }

    #[gpui_kit::test]
    fn a_note_the_server_refuses_stays_readable_with_why(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("emberhut"));
        let backend = Arc::new(FakeBackend::new([]));
        backend.days.push_save(Err(Error::Refused {
            status: Some(400),
            message: Some("That note can't be saved".into()),
        }));
        let store = cx.new(|_| {
            let mut store = Store::for_tests(backend.clone());
            store.files = Some(files.clone());
            store
        });
        store.update(cx, Store::start_diary);
        assert!(store.update(cx, |store, cx| store.save_note(
            "Keep these words",
            None,
            cx
        )));
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            let diary = store.diary();
            let [note] = diary.book().shown().try_into().unwrap();
            assert_eq!(note.text, "Keep these words", "still readable");
            assert!(!diary.book().has_unsent(), "not sent again as it is");
            assert_eq!(
                diary.book().refusal(&note.uid),
                Some("That note can't be saved")
            );
            assert_eq!(
                diary.problem(),
                Some("A note wasn't saved: That note can't be saved")
            );
            assert_eq!(
                store.action_problem(),
                Some("A note wasn't saved: That note can't be saved")
            );
        });
        assert_eq!(
            DiaryData::load(Some(&files)).book.shown()[0].text,
            "Keep these words",
            "kept across launches"
        );

        // Changed, it goes again, and the mark goes.
        let uid = store.read_with(cx, |store, _| store.diary().book().shown()[0].uid.clone());
        store.update(cx, |store, cx| {
            store.save_note("Fewer words", Some(&uid), cx)
        });
        store.read_with(cx, |store, _| {
            let book = store.diary().book();
            assert_eq!(book.refusal(&uid), None);
            assert!(book.is_unsent(&uid));
            assert_eq!(store.diary().problem(), None);
        });
    }

    #[gpui_kit::test]
    fn a_note_longer_than_the_server_takes_isnt_saved(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = cx.new(|_| Store::for_tests(backend.clone()));
        let long = "x".repeat(emberhut_api::notes::MAX_NOTE_LEN + 1);
        assert!(!store.update(cx, |store, cx| store.save_note(&long, None, cx)));
        let fits = format!("  {}  ", "x".repeat(emberhut_api::notes::MAX_NOTE_LEN));
        assert!(store.update(cx, |store, cx| store.save_note(&fits, None, cx)));
        store.read_with(cx, |store, _| {
            assert_eq!(store.diary().book().shown().len(), 1);
        });
    }

    #[test]
    fn the_diary_lasts_across_launches() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("emberhut"));
        assert_eq!(DiaryData::load(Some(&files)).book, Book::default());
        let mut book = Book::default();
        book.set_draft("Half".to_owned());
        write(&path(&files), &book);
        assert_eq!(DiaryData::load(Some(&files)).book.draft(), "Half");
        std::fs::write(path(&files), "{").unwrap();
        assert_eq!(DiaryData::load(Some(&files)).book, Book::default());
    }

    #[gpui_kit::test]
    fn a_note_shows_at_once_and_is_sent_then_the_diary_read(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend
            .days
            .push_save(Err(Error::Unreachable("offline".into())));
        let store = cx.new(|_| Store::for_tests(backend.clone()));
        store.update(cx, Store::start_diary);
        let saved = store.update(cx, |store, cx| store.save_note("Buy the part", None, cx));
        assert!(saved);
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            let book = store.diary().book();
            let [note] = book.shown().try_into().unwrap();
            assert_eq!(note.text, "Buy the part");
            assert!(book.is_unsent(&note.uid), "offline: kept to send");
        });

        let note = store.read_with(cx, |store, _| store.diary().book().shown()[0].clone());
        backend.days.push_notes(Ok(NotesPage {
            notes: vec![
                serde_json::from_value(json!({
                    "uid": note.uid, "who": "owner", "day": note.day, "at": note.at,
                    "text": "Buy the part", "created": note.at, "updated": note.at
                }))
                .unwrap(),
            ],
        }));
        cx.executor().advance_clock(RETRY);
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            let book = store.diary().book();
            assert!(!book.has_unsent(), "sent once the server could be reached");
            assert_eq!(book.shown().len(), 1, "the server's copy");
        });
        let sends = backend
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::SaveNote(_)))
            .count();
        assert_eq!(sends, 2, "tried, then sent again");
        assert!(backend.calls().contains(&Call::Notes));
    }
}
