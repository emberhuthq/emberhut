//! The Dark room's data: a plain chat with a model, off the record. A message's answer streams
//! in from the background as it is written ("Thinking…" before its first word); then the
//! room is read again for the kept messages. An answer still being written when the room
//! opens (from a message sent before the user left) is waited for, the room read every 2 s
//! until it is done.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::dark::{DarkEvent, DarkRole, DarkRoom};
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};
use crate::backend::Error;
use crate::brand::APP_NAME;

/// How often the room is read while an answer from before is still being written.
const BUSY_EVERY: Duration = Duration::from_secs(2);

/// A message in the room: the kept ones, and the user's own while its answer is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Said {
    /// The user's (else the model's).
    pub(crate) mine: bool,
    /// What it says.
    pub(crate) text: String,
}

/// The Dark room as this device has it.
#[derive(Debug)]
pub(crate) struct DarkData {
    chat: Vec<Said>,
    /// The model, in words.
    model: String,
    /// Where what the user writes goes, in words.
    place: String,
    /// The answer so far while it streams in (empty while the model thinks).
    writing: Option<String>,
    busy: bool,
    error: Option<String>,
    loaded: bool,
    /// A read every [`BUSY_EVERY`] is on its way.
    waiting: bool,
}

impl Default for DarkData {
    fn default() -> Self {
        Self {
            chat: Vec::new(),
            model: "DeepSeek".to_owned(),
            place: "DeepSeek on Cloudflare".to_owned(),
            writing: None,
            busy: false,
            error: None,
            loaded: false,
            waiting: false,
        }
    }
}

impl DarkData {
    /// What was said, oldest first.
    #[must_use]
    pub(crate) fn chat(&self) -> &[Said] {
        &self.chat
    }

    /// The model, in words.
    #[must_use]
    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    /// Where what the user writes goes, in words.
    #[must_use]
    pub(crate) fn place(&self) -> &str {
        &self.place
    }

    /// The answer so far, while one streams in (empty while the model thinks).
    #[must_use]
    pub(crate) fn writing(&self) -> Option<&str> {
        self.writing.as_deref()
    }

    /// Whether an answer is being written (nothing can be sent or cleared).
    #[must_use]
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    /// What went wrong last.
    #[must_use]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the empty room's note shows: read, nothing said, nothing being written.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.loaded && self.chat.is_empty() && self.writing.is_none()
    }

    /// Whether `draft` can be sent now.
    #[must_use]
    pub(crate) fn can_send(&self, draft: &str) -> bool {
        !self.busy && !draft.trim().is_empty()
    }

    /// Takes a read of the room in; whether to read it again in a moment (an answer from
    /// before is still being written).
    fn took(&mut self, room: Result<DarkRoom, Error>) -> bool {
        match room {
            Ok(room) => {
                self.chat = room
                    .chat
                    .into_iter()
                    .map(|message| Said {
                        mine: message.role == DarkRole::User,
                        text: message.text,
                    })
                    .collect();
                self.place = room
                    .place
                    .unwrap_or_else(|| format!("{} on Cloudflare", room.model));
                self.model = room.model;
                self.loaded = true;
                if self.writing.is_none() {
                    self.busy = room.busy;
                }
                room.busy && self.writing.is_none()
            }
            Err(error) => {
                log::warn!("couldn't read the Dark room: {error}");
                if !self.loaded {
                    self.error = Some(unread(&error));
                }
                false
            }
        }
    }

    /// `text` sent: it shows at once, and the answer is awaited.
    fn sent(&mut self, text: &str) {
        self.busy = true;
        self.error = None;
        self.writing = Some(String::new());
        self.chat.push(Said {
            mine: true,
            text: text.to_owned(),
        });
    }

    /// An event of the answer's stream.
    fn heard(&mut self, event: DarkEvent) {
        if let Some(delta) = event.delta {
            self.writing.get_or_insert_default().push_str(&delta);
        }
        if let Some(error) = event.error {
            self.error = Some(error);
        }
    }

    /// The answer's stream ended, well or not.
    fn ended(&mut self, end: Result<(), Error>) {
        if let Err(error) = end {
            log::warn!("the Dark room's answer failed: {error}");
            self.error = Some(match error {
                Error::SignInRequired => {
                    "This device has to sign in first; the message wasn't sent.".to_owned()
                }
                Error::Refused { message, .. } => {
                    message.unwrap_or_else(|| "The server said no.".to_owned())
                }
                _ => "The connection dropped; the answer is still kept when it's done.".to_owned(),
            });
        }
        self.busy = false;
        self.writing = None;
    }

    /// The room cleared, or why not.
    fn cleared(&mut self, outcome: Result<(), Error>) {
        match outcome {
            Ok(()) => {
                self.chat.clear();
                self.error = None;
            }
            Err(error) => {
                log::warn!("couldn't clear the Dark room: {error}");
                self.error = Some("Couldn't clear it now; try again in a moment.".to_owned());
            }
        }
    }
}

/// Why the room couldn't be read, worded for the user: a server that answers without the room
/// is not one that can't be reached.
fn unread(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}'s server."),
        Error::SignInRequired => "This device has to sign in first.".to_owned(),
        Error::Refused {
            message: Some(message),
            ..
        } => message.clone(),
        Error::Refused {
            status: Some(404), ..
        } => "Your server doesn't have the Dark room.".to_owned(),
        _ => "Couldn't read the Dark room; try again in a moment.".to_owned(),
    }
}

/// What the answer's stream tells the store.
enum Heard {
    /// An event.
    Event(DarkEvent),
    /// The stream ended.
    End(Result<(), Error>),
}

impl Store {
    /// The Dark room.
    #[must_use]
    pub(crate) fn dark(&self) -> &DarkData {
        &self.dark
    }

    /// Reads the Dark room; while an answer from before is still being written, again every
    /// [`BUSY_EVERY`] until it is done.
    pub(crate) fn load_dark(&mut self, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let room = cx.background_spawn(async move { backend.dark() }).await;
            let again = update_or_log(&this, cx, |store, cx| {
                let again = store.dark.took(room) && !store.dark.waiting;
                store.dark.waiting |= again;
                cx.notify();
                again
            });
            if again == Some(true) {
                cx.background_executor().timer(BUSY_EVERY).await;
                update_or_log(&this, cx, |store, cx| {
                    store.dark.waiting = false;
                    store.load_dark(cx);
                });
            }
        })
        .detach();
    }

    /// Writes `text` in the Dark room: it shows at once, and the answer streams in.
    pub(crate) fn send_dark(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        if !self.dark.can_send(text) {
            return;
        }
        let text = text.trim().to_owned();
        self.dark.sent(&text);
        let (heard, hearing) = smol::channel::unbounded::<Heard>();
        let backend = Arc::clone(&self.backend);
        // The answer may take minutes to write: it blocks a thread of its own
        // (`smol::unblock`), never one of the executor's few background threads, which the
        // rest of the app shares.
        smol::unblock(move || {
            let end = backend.send_dark(&text, &mut |event| {
                if heard.try_send(Heard::Event(event)).is_err() {
                    log::debug!("the Dark room's answer came after the app closed");
                }
            });
            if heard.try_send(Heard::End(end)).is_err() {
                log::debug!("the Dark room's answer ended after the app closed");
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(heard) = hearing.recv().await {
                let ended = matches!(heard, Heard::End(_));
                let gone = update_or_log(&this, cx, |store, cx| {
                    match heard {
                        Heard::Event(event) => store.dark.heard(event),
                        Heard::End(end) => {
                            store.dark.ended(end);
                            store.load_dark(cx);
                        }
                    }
                    cx.notify();
                })
                .is_none();
                if ended || gone {
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Deletes everything said in the Dark room.
    pub(crate) fn clear_dark(&mut self, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { backend.clear_dark() })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.dark.cleared(outcome);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn room(busy: bool, chat: &serde_json::Value) -> DarkRoom {
        serde_json::from_value(json!({"model": "DeepSeek V4.1 Flash", "busy": busy, "chat": chat}))
            .unwrap()
    }

    fn message(id: i64, role: &str, text: &str) -> serde_json::Value {
        json!({"id": id, "role": role, "text": text, "at": "2026-11-25T10:00:00.000Z"})
    }

    #[test]
    fn a_read_fills_the_room_and_names_where_words_go() {
        let mut dark = DarkData::default();
        assert!(!dark.is_empty(), "not read yet");
        let again = dark.took(Ok(room(
            false,
            &json!([message(1, "user", "Hi"), message(2, "assistant", "Hello")]),
        )));
        assert!(!again);
        assert_eq!(dark.place(), "DeepSeek V4.1 Flash on Cloudflare");
        assert_eq!(dark.model(), "DeepSeek V4.1 Flash");
        assert!(dark.chat()[0].mine && !dark.chat()[1].mine);
        dark.took(Ok(room(false, &json!([]))));
        assert!(dark.is_empty());
    }

    #[test]
    fn an_answer_from_before_is_waited_for() {
        let mut dark = DarkData::default();
        assert!(dark.took(Ok(room(true, &json!([message(1, "user", "Hi")])))));
        assert!(dark.is_busy());
        assert!(!dark.can_send("Again"));
        assert!(!dark.took(Ok(room(false, &json!([])))));
        assert!(dark.can_send("Again"));
        assert!(!dark.can_send("  "));
    }

    #[test]
    fn an_answer_streams_in_then_the_room_is_read_again() {
        let mut dark = DarkData::default();
        dark.took(Ok(room(false, &json!([]))));
        dark.sent("A name for a dog?");
        assert_eq!(dark.writing(), Some(""), "thinking");
        assert!(!dark.is_empty());
        assert!(
            !dark.took(Ok(room(true, &json!([])))),
            "not polled while it streams"
        );
        assert!(dark.is_busy(), "still busy: its own answer is on its way");
        dark.heard(DarkEvent {
            thinking: Some(true),
            ..DarkEvent::default()
        });
        dark.heard(DarkEvent {
            delta: Some("Bis".into()),
            ..DarkEvent::default()
        });
        dark.heard(DarkEvent {
            delta: Some("cuit.".into()),
            ..DarkEvent::default()
        });
        assert_eq!(dark.writing(), Some("Biscuit."));
        dark.ended(Ok(()));
        assert_eq!(dark.writing(), None);
        assert!(!dark.is_busy());
    }

    #[test]
    fn a_refusal_or_a_drop_says_so() {
        let mut dark = DarkData::default();
        dark.sent("Hi");
        dark.ended(Err(Error::Refused {
            status: Some(409),
            message: Some("A reply is still being written.".into()),
        }));
        assert_eq!(dark.error(), Some("A reply is still being written."));
        dark.sent("Hi");
        assert_eq!(dark.error(), None);
        dark.ended(Err(Error::Unreachable("reset".into())));
        assert!(dark.error().unwrap().contains("still kept"));
        dark.sent("Hi");
        dark.ended(Err(Error::SignInRequired));
        assert_eq!(
            dark.error(),
            Some("This device has to sign in first; the message wasn't sent.")
        );
        dark.heard(DarkEvent {
            error: Some("Workers AI 500".into()),
            ..DarkEvent::default()
        });
        assert_eq!(dark.error(), Some("Workers AI 500"));
    }

    #[test]
    fn an_unreachable_room_says_so_only_before_its_first_read() {
        let mut dark = DarkData::default();
        dark.took(Err(Error::Unreachable("down".into())));
        assert_eq!(dark.error(), Some("Can't reach Emberhut's server."));
        let mut read = DarkData::default();
        read.took(Ok(room(false, &json!([]))));
        read.took(Err(Error::Unreachable("down".into())));
        assert_eq!(read.error(), None);
    }

    #[test]
    fn a_server_without_the_room_isnt_called_unreachable() {
        let read = |error| {
            let mut dark = DarkData::default();
            dark.took(Err(error));
            dark.error().map(str::to_owned)
        };
        assert_eq!(
            read(Error::Refused {
                status: Some(404),
                message: None
            })
            .as_deref(),
            Some("Your server doesn't have the Dark room.")
        );
        assert_eq!(
            read(Error::Refused {
                status: Some(503),
                message: Some("The model is busy.".into())
            })
            .as_deref(),
            Some("The model is busy."),
            "the server's own words come first"
        );
        assert_eq!(
            read(Error::SignInRequired).as_deref(),
            Some("This device has to sign in first.")
        );
    }

    #[test]
    fn clearing_empties_the_room_or_says_why_not() {
        let mut dark = DarkData::default();
        dark.took(Ok(room(false, &json!([message(1, "user", "Hi")]))));
        dark.cleared(Err(Error::Refused {
            status: Some(409),
            message: None,
        }));
        assert_eq!(dark.chat().len(), 1);
        assert!(dark.error().unwrap().starts_with("Couldn't clear it now"));
        dark.cleared(Ok(()));
        assert_eq!(dark.chat(), []);
        assert_eq!(dark.error(), None);
    }

    mod glue {
        use std::sync::Arc;

        use std::time::Duration;

        use gpui_kit::{Entity, TestAppContext};

        use super::*;
        use crate::backend::Backend;
        use crate::backend::fake::{Call, FakeBackend};
        use crate::store::party::roster::tests::page;
        use crate::store::party::tests::settle;

        fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
            let backend = Arc::clone(backend) as Arc<dyn Backend>;
            cx.new(|_| Store::for_tests(backend))
        }

        #[gpui_kit::test]
        fn the_room_is_read_and_cleared(cx: &mut TestAppContext) {
            let backend = Arc::new(FakeBackend::new([]));
            backend
                .party
                .push_dark(Ok(room(false, &json!([message(1, "user", "Hi")]))));
            let store = store(cx, &backend);
            store.update(cx, Store::load_dark);
            cx.run_until_parked();
            store.read_with(cx, |store, _| assert_eq!(store.dark().chat().len(), 1));
            backend.party.push_clear(Ok(()));
            store.update(cx, Store::clear_dark);
            cx.run_until_parked();
            store.read_with(cx, |store, _| assert_eq!(store.dark().chat(), []));
            assert_eq!(
                backend.calls(),
                [Call::Dark, Call::ClearDark],
                "one read, one clear"
            );
        }

        #[gpui_kit::test]
        fn a_message_streams_its_answer_then_the_room_is_read(cx: &mut TestAppContext) {
            // The answer blocks a thread of its own, which wakes this test's tasks from outside it.
            cx.executor().allow_parking();
            let backend = Arc::new(FakeBackend::new([]));
            let delta = |text: &str| DarkEvent {
                delta: Some(text.into()),
                ..DarkEvent::default()
            };
            backend
                .party
                .push_answer(vec![delta("Bis"), delta("cuit.")], Ok(()));
            backend.party.push_dark(Ok(room(
                false,
                &json!([
                    message(1, "user", "A dog?"),
                    message(2, "assistant", "Biscuit.")
                ]),
            )));
            let store = store(cx, &backend);
            store.update(cx, |store, cx| {
                store.send_dark("  ", cx);
                store.send_dark(" A dog? ", cx);
                assert_eq!(store.dark().writing(), Some(""));
                assert_eq!(store.dark().chat()[0].text, "A dog?");
            });
            settle(cx, Duration::ZERO, |cx| {
                store.read_with(cx, |store, _| store.dark().chat().len() == 2)
            });
            store.read_with(cx, |store, _| {
                let dark = store.dark();
                assert_eq!(dark.chat()[1].text, "Biscuit.");
                assert!(dark.writing().is_none() && !dark.is_busy());
            });
            assert_eq!(
                backend.calls(),
                [Call::SendDark("A dog?".into()), Call::Dark],
                "a blank message isn't sent"
            );
        }

        #[gpui_kit::test]
        fn an_answer_written_for_long_holds_none_of_the_apps_background_work(
            cx: &mut TestAppContext,
        ) {
            // The answer blocks a thread of its own, which wakes this test's tasks from outside it.
            cx.executor().allow_parking();
            let backend = Arc::new(FakeBackend::new([]));
            let go_on = backend.party.hold_answers();
            backend.party.push_parties(Ok(page()));
            let store = store(cx, &backend);
            store.update(cx, |store, cx| {
                store.send_dark("A dog?", cx);
                store.load_parties(cx);
            });
            cx.run_until_parked();
            store.read_with(cx, |store, _| {
                assert!(store.dark().is_busy(), "the answer is still being written");
                assert_eq!(
                    store.party().roster().parties().len(),
                    2,
                    "and the parties were read meanwhile"
                );
            });
            go_on.send(()).unwrap();
            settle(cx, Duration::ZERO, |cx| {
                store.read_with(cx, |store, _| !store.dark().is_busy())
            });
        }
    }
}
