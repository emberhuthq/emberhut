//! Telling the server what the user reads: the window says when it is the active one and when
//! the user uses it, and the open room's new answers are marked seen
//! ([`SeenMarks`](super::reading::SeenMarks) decides when) once its chat is in front of them.

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::AppContext as _;
use gpui_kit::{App, Context};

use super::presence::Presence;
use super::reading::{OnScreen, SeenMark};
use super::{Store, update_or_log};
use crate::backend::Error;

impl Store {
    /// What is in front of the user at `now`.
    #[must_use]
    pub(super) fn on_screen(&self, now: Instant) -> OnScreen {
        OnScreen {
            room: self.room,
            signing_in: self.shows_sign_in(),
            window: self.presence(),
            at_keyboard: self.last_input.is_recent(now),
        }
    }

    /// The window became the active one, or stopped being it: a room on screen is read from
    /// now on, or no longer. Bringing the window to the front is the user's doing, so it counts
    /// as input. The poll and the machines go by it too ([`Presence`]): come to the front, both
    /// go at once ([`presence_changed`](Self::presence_changed)), and the poll says the room is
    /// looked at.
    pub(crate) fn set_window_active(&mut self, active: bool, cx: &mut Context<'_, Self>) {
        if self.window.active == active {
            return;
        }
        let before = self.presence();
        self.window.active = active;
        self.presence_changed(before);
        if active {
            self.last_input.input(now(cx));
            self.mark_read(cx);
        }
    }

    /// The user used the window (a key, a click, a scroll, the pointer). Back after being
    /// away, what is on screen is read again: the poll says so at once.
    pub(crate) fn note_input(&mut self, cx: &mut Context<'_, Self>) {
        if self.last_input.input(now(cx)) && self.presence() == Presence::Front {
            self.kicker.kick();
            self.mark_read(cx);
        }
    }

    /// The room the user reads now, if any.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn reading(&self, cx: &App) -> Option<emberhut_api::ids::RoomId> {
        self.on_screen(now(cx)).reading()
    }

    /// Marks the open room's answers seen if the user is reading them and the server doesn't
    /// know yet; once it does, the poll goes at once to bring the cleared unread.
    pub(super) fn mark_read(&mut self, cx: &mut Context<'_, Self>) {
        let Some(state) = &self.state else {
            return;
        };
        let reading = self.on_screen(now(cx)).reading();
        let Some(mark) = self.seen_marks.due(reading, state) else {
            return;
        };
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let sent = cx
                .background_spawn(async move { backend.seen(mark.room) })
                .await;
            update_or_log(&this, cx, |store, _| store.marked(mark, sent));
        })
        .detach();
    }

    /// How marking `mark` seen went. A mark that didn't reach the server is sent again with
    /// the next poll that does; a refused one isn't (newer answers are tried).
    fn marked(&mut self, mark: SeenMark, sent: Result<(), Error>) {
        match sent {
            Ok(()) => self.kicker.kick(),
            Err(error) => {
                if self.seen_marks.failed(mark, error.is_unreachable()) {
                    log::warn!("couldn't mark room {} seen: {error}", mark.room);
                } else {
                    log::debug!("still couldn't mark room {} seen: {error}", mark.room);
                }
            }
        }
    }
}

/// The time now, on GPUI's clock (a test's moves only when the test says).
fn now(cx: &App) -> Instant {
    cx.background_executor().now()
}
