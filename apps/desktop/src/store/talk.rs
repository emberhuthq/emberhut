//! A message to the assistant, with the files attached in the composer: shown at once as
//! pending, with a note on how it went, until the server's chat has it (one the server took
//! without starting a turn keeps its bubble with no note). A failed one is sent
//! again with a click on its note. One still without an answer after [`LOST_AFTER`], which the
//! chat doesn't show either, says so and waits for that click too (never sent again by itself:
//! a copy that did arrive late would reach the assistant twice); once the server has answered
//! that it has the message, it is never called lost. Quitting waits a little for a message on
//! its way, as for answers.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::composer::reason;
use super::{Store, update_or_log};
use crate::backend::Error;
use crate::brand::APP_NAME;
use emberhut_api::chat::{ChatFile, ChatMessage, Quote};
use emberhut_api::talk::{TalkReply, TalkRequest};

/// How long a message may go without an answer, and without the chat having it, before its
/// note says it was lost.
pub(crate) const LOST_AFTER: Duration = Duration::from_secs(45);

/// How a message sent from here is going.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Stage {
    /// On its way: the server hasn't answered yet.
    #[default]
    Sending,
    /// The server has it, and the assistant answers.
    Typing,
    /// The server has it and started no turn (one is running, say): the bubble stays, with no
    /// note, until the chat shows the message.
    Taken,
    /// The server refused it or couldn't be reached: why, worded for the user.
    Failed(String),
    /// No answer came within [`LOST_AFTER`], and the chat doesn't show it.
    Lost,
}

impl Stage {
    /// The stage a send's answer leaves it at.
    #[must_use]
    pub(crate) fn of(reply: &Result<TalkReply, Error>) -> Self {
        match reply {
            Ok(reply) if reply.ok && reply.started => Self::Typing,
            Ok(reply) if reply.ok => Self::Taken,
            Ok(_) => Self::Failed("no reason given".to_owned()),
            Err(error) => Self::Failed(reason(error)),
        }
    }

    /// Whether it didn't go, so a click on its note sends it again.
    #[must_use]
    pub(crate) fn failed(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Lost)
    }

    /// Whether the answer the assistant writes for it is still to come, so the stop button
    /// drops it.
    #[must_use]
    pub(crate) fn is_on_its_way(&self) -> bool {
        matches!(self, Self::Sending | Self::Typing)
    }

    /// What its note says: none once the server took it without a turn.
    #[must_use]
    pub(crate) fn note(&self) -> Option<String> {
        match self {
            Self::Sending => Some("Sending…".to_owned()),
            Self::Typing => Some(format!("{APP_NAME} is typing…")),
            Self::Taken => None,
            Self::Failed(why) => Some(format!("Not sent: {why}")),
            Self::Lost => Some(format!("Not sent: {APP_NAME} never got it")),
        }
    }
}

/// A message sent from here that the chat doesn't show yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingMessage {
    /// What the user wrote.
    pub(crate) text: String,
    /// The files that go with it.
    pub(crate) files: Vec<ChatFile>,
    /// How it is going.
    pub(crate) stage: Stage,
    /// The message it replies to.
    pub(crate) reply: Option<Quote>,
    /// Which send this is, so that a late answer or timer for an earlier one changes nothing.
    pub(crate) attempt: u64,
}

impl PendingMessage {
    /// Whether `chat` has it now: one of the user's messages with its text and, when it has
    /// files, one of them (the server drops files it didn't store).
    #[must_use]
    pub(crate) fn is_in(&self, chat: &[ChatMessage]) -> bool {
        chat.iter().any(|message| {
            message.mine
                && message.text.trim() == self.text.trim()
                && (self.files.is_empty()
                    || message
                        .files
                        .iter()
                        .any(|file| self.files.iter().any(|ours| ours.url == file.url)))
        })
    }

    /// Whether it failed, so a click on its note sends it again.
    #[must_use]
    pub(crate) fn failed(&self) -> bool {
        self.stage.failed()
    }

    /// What its note says: none once the server took it without a turn.
    #[must_use]
    pub(crate) fn note(&self) -> Option<String> {
        self.stage.note()
    }

    /// What its note says when it is a button: the failure, and that a click sends it again.
    #[must_use]
    pub(crate) fn resend_label(&self) -> String {
        let note = self.note().unwrap_or_default();
        format!("{note} · tap to send again")
    }
}

impl Store {
    /// A new page of the open room's chat: the pending line goes once the chat has it.
    pub(super) fn settle_pending(&mut self, chat: &[ChatMessage]) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.is_in(chat))
        {
            self.pending = None;
        }
    }

    /// Sends `text` and the stored attachments to the assistant in the open room: only once no
    /// upload is running, and only with text or a file. Whether it went (the composer empties
    /// only then). A quote waiting above the composer makes it a reply.
    pub(crate) fn talk(&mut self, text: &str, cx: &mut Context<'_, Self>) -> bool {
        let text = text.trim().to_owned();
        if !self.attachments.can_send(&text) {
            return false;
        }
        let files = self.attachments.take_for_send();
        let reply = self.replying.take();
        self.send_message(text, files, reply, cx);
        true
    }

    /// Says `text` in the open room as it is: no attachments, not a reply (the message menu's
    /// "Explain simply").
    pub(crate) fn say(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        let text = text.trim().to_owned();
        if !text.is_empty() {
            self.send_message(text, Vec::new(), None, cx);
        }
    }

    /// A click on a failed message's note: the same message (its files and its reply too)
    /// goes again.
    pub(crate) fn resend(&mut self, cx: &mut Context<'_, Self>) {
        let Some(pending) = self.pending.take_if(|pending| pending.failed()) else {
            return;
        };
        self.send_message(pending.text, pending.files, pending.reply, cx);
    }

    /// Shows the message as pending and sends it. The request starts at once on a background
    /// thread and counts as in flight, so quitting waits for it.
    fn send_message(
        &mut self,
        text: String,
        files: Vec<ChatFile>,
        reply: Option<Quote>,
        cx: &mut Context<'_, Self>,
    ) {
        self.sends += 1;
        let attempt = self.sends;
        let mut request = TalkRequest::text(self.room, text.clone()).with_files(files.clone());
        if let Some(quote) = &reply {
            request = request.with_reply_to(quote.id);
        }
        self.pending = Some(PendingMessage {
            text,
            files,
            stage: Stage::Sending,
            reply,
            attempt,
        });
        cx.notify();
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        let sending = self.in_flight.start();
        let reply = cx.background_spawn(async move {
            let reply = backend.talk(&request);
            drop(sending);
            reply
        });
        cx.spawn(async move |this, cx| {
            let reply = reply.await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                // Only the note of this send: the user may have switched rooms or sent another.
                if let Some(pending) = store.pending_attempt(attempt) {
                    pending.stage = Stage::of(&reply);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOST_AFTER).await;
            update_or_log(&this, cx, |store, cx| store.check_lost(attempt, cx));
        })
        .detach();
    }

    /// The pending message, if it is still send `attempt`.
    fn pending_attempt(&mut self, attempt: u64) -> Option<&mut PendingMessage> {
        self.pending
            .as_mut()
            .filter(|pending| pending.attempt == attempt)
    }

    /// [`LOST_AFTER`] went by since send `attempt`: if the server still hasn't answered and the
    /// open room's chat doesn't have it, its note says it was lost. One the server answered
    /// for is never lost: it has it.
    pub(super) fn check_lost(&mut self, attempt: u64, cx: &mut Context<'_, Self>) {
        let room = self.room;
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        if pending.attempt != attempt
            || pending.stage != Stage::Sending
            || pending.is_in(self.chats.messages(room))
        {
            return;
        }
        if let Some(pending) = self.pending_attempt(attempt) {
            pending.stage = Stage::Lost;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::message_at;

    #[test]
    fn the_stage_says_how_sending_went() {
        let reply = |ok, started| {
            Ok(TalkReply {
                ok,
                started,
                id: None,
            })
        };
        assert_eq!(Stage::of(&reply(true, true)), Stage::Typing);
        assert_eq!(
            Stage::of(&reply(true, true)).note().as_deref(),
            Some("Emberhut is typing…")
        );
        assert_eq!(Stage::of(&reply(true, false)), Stage::Taken);
        assert_eq!(Stage::of(&reply(true, false)).note(), None, "no note line");
        assert_eq!(
            Stage::of(&reply(false, false)).note().as_deref(),
            Some("Not sent: no reason given")
        );
        let refused = Err(Error::Refused {
            status: Some(500),
            message: Some("Too long".into()),
        });
        assert_eq!(
            Stage::of(&refused).note().as_deref(),
            Some("Not sent: Too long")
        );
        let failed = Err(Error::Refused {
            status: Some(500),
            message: None,
        });
        assert_eq!(
            Stage::of(&failed).note().as_deref(),
            Some("Not sent: the server said no (500)")
        );
    }

    #[test]
    fn a_pending_message_is_in_the_chat_once_the_user_said_it() {
        let pending = PendingMessage {
            text: "Hello ".into(),
            ..PendingMessage::default()
        };
        let theirs = message_at(1, "2026-01-01T00:00:00Z", json!({"text": "Hello"}));
        let mine = message_at(
            2,
            "2026-01-01T00:00:00Z",
            json!({"text": "Hello", "mine": true}),
        );
        assert!(!pending.is_in(std::slice::from_ref(&theirs)));
        assert!(pending.is_in(&[theirs, mine]));
    }

    #[test]
    fn a_pending_message_with_files_is_the_one_with_them() {
        let file = json!({"name": "a.png", "url": "/uploads/1-a.png", "kind": "image",
            "path": "/data/uploads/1-a.png"});
        let pending = PendingMessage {
            files: vec![serde_json::from_value(file.clone()).unwrap()],
            ..PendingMessage::default()
        };
        let earlier = message_at(
            1,
            "2026-01-01T00:00:00Z",
            json!({"text": "", "mine": true, "files": [{"name": "b.png", "url": "/uploads/2-b.png",
                "kind": "image", "path": "/data/uploads/2-b.png"}]}),
        );
        let this_one = message_at(
            2,
            "2026-01-01T00:00:00Z",
            json!({"text": "", "mine": true, "files": [file]}),
        );
        assert!(!pending.is_in(std::slice::from_ref(&earlier)));
        assert!(pending.is_in(&[earlier, this_one]));
    }

    #[test]
    fn a_failed_message_is_a_button_that_sends_it_again() {
        let failed = PendingMessage {
            stage: Stage::Failed("send again later, the server said".into()),
            ..PendingMessage::default()
        };
        assert!(failed.failed());
        assert_eq!(
            failed.resend_label(),
            "Not sent: send again later, the server said · tap to send again",
            "the server's words never stand for the button's"
        );
        let lost = PendingMessage {
            stage: Stage::Lost,
            ..PendingMessage::default()
        };
        assert!(lost.failed());
        assert_eq!(
            lost.resend_label(),
            "Not sent: Emberhut never got it · tap to send again"
        );
        for stage in [Stage::Sending, Stage::Typing, Stage::Taken] {
            assert!(!stage.failed(), "{stage:?}");
        }
        assert!(Stage::Typing.is_on_its_way() && Stage::Sending.is_on_its_way());
        assert!(!Stage::Taken.is_on_its_way() && !Stage::Lost.is_on_its_way());
    }
}
