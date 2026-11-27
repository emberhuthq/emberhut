//! What the composer holds beyond its field: each room's unsent words, words put in from
//! elsewhere (a starter, a Talk button), a call's recording being started or stopped, and the
//! last action of the composer's that failed (a stop, a call's recording), as a line over the
//! field until another room opens. The message being replied to is [`super::replying`]'s.
//!
//! The field itself lives in the chat view; it hands its words here when the room changes and
//! takes the next room's back.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::calls::{RecordAction, RecordCallRequest};
use emberhut_api::ids::RoomId;
use emberhut_api::state::State;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};
use crate::backend::Error;
use crate::brand::APP_NAME;
use crate::dictation::mic::Recording;
use crate::dictation::{self, FILE_NAME, Unheard, WORDS};

/// How long a start or stop of a call's recording the server took may go unseen in the state
/// before the composer stops waiting for it.
pub(crate) const CALL_SETTLES: Duration = Duration::from_secs(10);

/// A start or stop of a call's recording that the state doesn't show yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CallChange {
    /// What was asked.
    action: RecordAction,
    /// Whether the server has said it told the Mac.
    answered: bool,
    /// Which ask this is, so that a late answer or timer for an earlier one changes nothing.
    attempt: u64,
}

/// The composer's own state.
#[derive(Debug, Default)]
pub(crate) struct Composer {
    /// Each room's words not sent yet, kept while another room is open.
    drafts: HashMap<RoomId, String>,
    /// Words put in from elsewhere, for the field to take.
    started: Option<String>,
    /// A call's recording being started or stopped.
    call_change: Option<CallChange>,
    /// The asks to start or stop a call's recording so far, numbering each.
    call_asks: u64,
    /// The last of the composer's actions that failed, worded for the user.
    notice: Option<String>,
}

impl Composer {
    /// The room changes from `from` to `to` with `text` in the field: `text` is kept as
    /// `from`'s draft and `to`'s comes back (empty when it has none). A failed action's line
    /// was about the room left, so it goes.
    pub(crate) fn switch(&mut self, from: RoomId, to: RoomId, text: &str) -> String {
        self.notice = None;
        if text.trim().is_empty() {
            self.drafts.remove(&from);
        } else {
            self.drafts.insert(from, text.to_owned());
        }
        self.drafts.remove(&to).unwrap_or_default()
    }

    /// Words put in from elsewhere, taken once.
    pub(crate) fn take_started(&mut self) -> Option<String> {
        self.started.take()
    }

    /// The last failed action, worded for the user.
    #[must_use]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Whether a room other than the open one keeps unsent words.
    #[must_use]
    pub(crate) fn has_drafts(&self) -> bool {
        !self.drafts.is_empty()
    }
}

/// The field's prompt: `Message Emberhut` in Main, `Message Emberhut in <room>` in another room.
#[must_use]
pub(crate) fn placeholder(state: Option<&State>, room: RoomId) -> String {
    state
        .and_then(|state| state.rooms.iter().find(|row| row.id == room))
        .filter(|row| !row.is_main() && !row.name.trim().is_empty())
        .map_or_else(
            || format!("Message {APP_NAME}"),
            |row| format!("Message {APP_NAME} in {}", row.name.trim()),
        )
}

/// Why `error` happened, for the user: the server's words when it gave them, else ours.
#[must_use]
pub(crate) fn reason(error: &Error) -> String {
    error
        .server_message()
        .map_or_else(|| error.to_string(), str::to_owned)
}

/// What a failed action says: `<what>: <the server's words, or ours>`.
#[must_use]
pub(crate) fn failure(what: &str, error: &Error) -> String {
    format!("{what}: {}", reason(error))
}

impl Store {
    /// The composer's own state.
    #[must_use]
    pub(crate) fn composer(&self) -> &Composer {
        &self.composer
    }

    /// The room changed under the field: see [`Composer::switch`]. A reply is to a message of
    /// the room left, so it goes.
    pub(crate) fn switch_draft(&mut self, from: RoomId, to: RoomId, text: &str) -> String {
        self.replying = None;
        self.composer.switch(from, to, text)
    }

    /// Words put in from elsewhere (a starter, Talk on a task's page or in another window, a
    /// `emberhut://chat?text=` link): the open room's composer holds `text` in place of what it
    /// held, focused, beside whatever the column shows.
    pub(crate) fn start_message(&mut self, text: impl Into<String>, cx: &mut Context<'_, Self>) {
        self.composer.started = Some(text.into());
        cx.notify();
    }

    /// Takes the words put in from elsewhere, once.
    pub(crate) fn take_started(&mut self) -> Option<String> {
        self.composer.take_started()
    }

    /// Hides the line about a failed action.
    pub(crate) fn dismiss_notice(&mut self, cx: &mut Context<'_, Self>) {
        self.composer.notice = None;
        cx.notify();
    }

    /// Says an action failed, on the line over the composer.
    pub(super) fn notify_failure(&mut self, notice: String, cx: &mut Context<'_, Self>) {
        log::warn!("{notice}");
        self.composer.notice = Some(notice);
        cx.notify();
    }

    /// The stop button: the assistant drops the answer it is writing in the open room (its
    /// background work goes on). The message on its way to that answer goes with it; a failed
    /// one stays, to be sent again.
    pub(crate) fn interrupt(&mut self, cx: &mut Context<'_, Self>) {
        let room = self.room;
        self.pending
            .take_if(|pending| pending.stage.is_on_its_way());
        self.composer.notice = None;
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let stopped = cx
                .background_spawn(async move { backend.interrupt(room) })
                .await;
            kicker.kick();
            if let Err(error) = stopped {
                update_or_log(&this, cx, |store, cx| {
                    store.notify_failure(failure(&format!("Couldn't stop {APP_NAME}"), &error), cx);
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// The words the speech-to-text is told to expect: the app's, and the rooms' names.
    fn dictation_words(&self) -> Vec<String> {
        let rooms = self.state().map_or(&[][..], |state| state.rooms.as_slice());
        WORDS
            .iter()
            .map(|&word| word.to_owned())
            .chain(
                rooms
                    .iter()
                    .filter(|room| !room.is_main())
                    .map(|room| room.name.trim().to_owned())
                    .filter(|name| !name.is_empty()),
            )
            .collect()
    }

    /// Finishes a dictation's recording, makes its WAV and turns it into text on the server,
    /// all off the UI thread: what was said, or why nothing was.
    pub(crate) fn transcribe(
        &self,
        recording: Box<dyn Recording>,
        cx: &mut Context<'_, Self>,
    ) -> gpui_kit::Task<Result<String, Unheard>> {
        let backend = Arc::clone(&self.backend);
        let words = self.dictation_words();
        cx.background_spawn(async move {
            let wav = dictation::prepare(&recording.finish())?;
            backend
                .transcribe(FILE_NAME, &words, &wav)
                .map_err(|error| {
                    log::warn!("dictation failed: {error}");
                    Unheard::Failed(error)
                })
        })
    }

    /// The start or stop of a call's recording asked for and not shown by the state yet:
    /// while there is one, the composer says so and asks for no other.
    #[must_use]
    pub(crate) fn call_change(&self) -> Option<RecordAction> {
        let change = self.composer.call_change?;
        let recording = self.state().is_some_and(|state| state.recording.is_some());
        let shown = recording == (change.action == RecordAction::Start);
        (!change.answered || !shown).then_some(change.action)
    }

    /// Starts recording a call on the Mac the server works with (its microphone and its sound),
    /// the transcript going to the open room; or stops the recording running. Nothing while
    /// another start or stop is on its way or not shown yet.
    pub(crate) fn record_call(&mut self, action: RecordAction, cx: &mut Context<'_, Self>) {
        if self.call_change().is_some() {
            return;
        }
        self.composer.call_asks += 1;
        let attempt = self.composer.call_asks;
        self.composer.call_change = Some(CallChange {
            action,
            answered: false,
            attempt,
        });
        self.composer.notice = None;
        let request = RecordCallRequest {
            action,
            room: self.room,
        };
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { backend.record_call(&request) })
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                store.call_answered(attempt, done, cx);
            });
            cx.background_executor().timer(CALL_SETTLES).await;
            update_or_log(&this, cx, |store, cx| {
                if store
                    .composer
                    .call_change
                    .take_if(|change| change.attempt == attempt)
                    .is_some()
                {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The server answered ask `attempt`: the composer waits for the state to show it, or says
    /// why it failed.
    fn call_answered(&mut self, attempt: u64, done: Result<(), Error>, cx: &mut Context<'_, Self>) {
        let Some(action) = self
            .composer
            .call_change
            .filter(|change| change.attempt == attempt)
            .map(|change| change.action)
        else {
            return;
        };
        match done {
            Ok(()) => {
                if let Some(change) = self.composer.call_change.as_mut() {
                    change.answered = true;
                }
            }
            Err(error) => {
                let what = match action {
                    RecordAction::Start => "Couldn't start recording",
                    RecordAction::Stop => "Couldn't stop recording",
                };
                self.composer.call_change = None;
                self.notify_failure(failure(what, &error), cx);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{room, state};

    #[test]
    fn each_room_keeps_its_own_draft() {
        let mut composer = Composer::default();
        let other = RoomId::new(2);
        assert_eq!(composer.switch(RoomId::MAIN, other, "half a thought"), "");
        assert_eq!(
            composer.switch(other, RoomId::MAIN, "for room 2"),
            "half a thought"
        );
        assert_eq!(composer.switch(RoomId::MAIN, other, "  "), "for room 2");
        assert_eq!(
            composer.switch(other, RoomId::MAIN, ""),
            "",
            "a blank field keeps no draft"
        );
    }

    #[test]
    fn started_words_are_taken_once() {
        let mut composer = Composer {
            started: Some("Check on the build".into()),
            ..Composer::default()
        };
        assert_eq!(
            composer.take_started().as_deref(),
            Some("Check on the build")
        );
        assert_eq!(composer.take_started(), None);
    }

    #[test]
    fn the_prompt_names_the_room() {
        let state = state(json!({
            "rooms": [room(1, json!({"name": "Main"})), room(2, json!({"name": "Room 2"}))]
        }));
        assert_eq!(placeholder(Some(&state), RoomId::MAIN), "Message Emberhut");
        assert_eq!(
            placeholder(Some(&state), RoomId::new(2)),
            "Message Emberhut in Room 2"
        );
        assert_eq!(
            placeholder(Some(&state), RoomId::new(3)),
            "Message Emberhut"
        );
        assert_eq!(placeholder(None, RoomId::new(2)), "Message Emberhut");
    }

    #[test]
    fn a_failure_says_what_and_why() {
        let refused = Error::Refused {
            status: None,
            message: Some("the Mac isn't reachable".into()),
        };
        assert_eq!(
            failure("Couldn't start recording", &refused),
            "Couldn't start recording: the Mac isn't reachable"
        );
        let unreachable = Error::Unreachable("refused".into());
        assert_eq!(
            failure("Couldn't stop Emberhut", &unreachable),
            "Couldn't stop Emberhut: can't reach the server"
        );
    }
}
