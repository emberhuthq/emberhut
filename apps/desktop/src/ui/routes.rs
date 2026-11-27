//! What a main window does for a link, a menu or another window: open a task or a page in its
//! column, open Settings on one of its pages, start a message in its chat.

use emberhut_api::ids::TaskId;
use gpui_kit::prelude::*;
use gpui_kit::{App, Window};

use super::column;
use super::window::MainWindow;
use crate::store::Page;
use crate::store::gate::Gate;
use crate::store::settings::SettingsPane;

impl MainWindow {
    /// Opens task `task`'s page in this window's column, over what it shows (Back returns
    /// there), or alone when the column is shut (Back returns to the chat).
    pub(crate) fn open_task(
        &mut self,
        task: TaskId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        log::info!("opening task {task}");
        self.push(Page::Task(task), window, cx);
    }

    /// Opens `page` over what this window's column shows (Back returns there), as a link to a
    /// page does; with the column shut it opens alone.
    pub(crate) fn push(&mut self, page: Page, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.column.update(cx, |column, cx| column.push(page, cx));
        self.keep_focus(window, cx);
    }

    /// Starts a message with `text` in the chat's composer, focused.
    pub(crate) fn start_message(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.store.read(cx).gate() != Gate::Open {
            log::info!("not starting a message behind the sign-in screen");
            return;
        }
        self.chat
            .update(cx, |chat, cx| chat.start_message(text, window, cx));
    }

    /// Puts the keyboard focus in the chat's composer, when the chat is on screen (behind the
    /// sign-in screen, or with the column in the chat's place, the window keeps it, so the
    /// shortcuts still reach it).
    pub(crate) fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let chat_shows = !self.column_shows(cx) || self.column_width(window, cx).is_some();
        if self.store.read(cx).gate() == Gate::Open && chat_shows {
            self.chat
                .update(cx, |chat, cx| chat.focus_composer(window, cx));
        } else {
            self.keep_focus(window, cx);
        }
    }
}

/// Opens Settings afresh in `window`'s column, with `pane` over it when one is named (Back
/// returns to Settings).
pub(crate) fn open_settings(pane: Option<SettingsPane>, window: &Window, cx: &mut App) {
    column::open(Page::Settings, window, cx);
    if let Some(pane) = pane {
        column::push(Page::SettingsPane(pane), window, cx);
    }
}
