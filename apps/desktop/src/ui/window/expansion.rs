//! The report expanded over the page: how it expands and comes back, and what brings it back on
//! its own. Choosing a page or a room, even the one showing, restores the panel
//! (`MainWindow::open` and the room shortcuts). So does whatever the user doesn't choose: another
//! screen in place of the app (sign-in, connecting, "not your device"), the report closing,
//! another room, and a page the column shows (a link, the automation's steps). Nothing changes
//! out of sight.
//!
//! Expanding takes the page away, and with it the field that had the keyboard focus, so the
//! window takes the focus. Coming back, by any route, gives it to the composer when the page is
//! a chat.

use emberhut_api::ids::RoomId;
use gpui_kit::prelude::*;
use gpui_kit::{Entity, Window};

use super::MainWindow;
use crate::store::gate::Gate;
use crate::store::{Page, Store};
use crate::ui::report;

/// What the window shows besides the report, as the store and the window's column have it:
/// when any of it changes, an expanded report comes back to its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Beside {
    room: RoomId,
    column: Option<Page>,
    gate: Gate,
    report_open: bool,
}

impl Beside {
    /// What `store` has the window show, its column showing `column`.
    #[must_use]
    pub(super) fn of(store: &Store, column: Option<Page>) -> Self {
        Self {
            room: store.room(),
            column,
            gate: store.gate(),
            report_open: store.open_report().is_some(),
        }
    }
}

impl MainWindow {
    /// Restores the report whenever what the window shows besides it changes in `store`.
    pub(super) fn follow_for_report(
        store: &Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        cx.observe_in(store, window, |this, _, window, cx| {
            this.note_beside(window, cx);
        })
        .detach();
    }

    /// What the window shows besides the report may have changed: if it did, an expanded
    /// report comes back.
    pub(super) fn note_beside(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let beside = Beside::of(self.store.read(cx), self.column.read(cx).current());
        if beside != self.report_beside {
            self.report_beside = beside;
            self.restore_report(window, cx);
        }
    }

    /// Lets the open report fill the window beside the sidebar, or brings it back to its width.
    pub(in crate::ui) fn toggle_report_expanded(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.report_size.is_expanded() {
            self.restore_report(window, cx);
            return;
        }
        if self.store.read(cx).open_report().is_none() {
            return;
        }
        let space = report::space(self, window, cx);
        self.report_size.expand(space, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Brings an expanded report back to its width (the one way back, whatever the route), the
    /// keyboard focus to the composer when the page is a chat in the app (no other screen
    /// has a composer).
    pub(super) fn restore_report(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.report_size.is_expanded() {
            return;
        }
        self.report_size.restore();
        // No page in the column: the chat is the page.
        let chat = self.column.read(cx).current().is_none();
        if chat && self.store.read(cx).gate() == Gate::Open {
            self.chat
                .update(cx, |chat, cx| chat.focus_composer(window, cx));
        }
        cx.notify();
    }
}
