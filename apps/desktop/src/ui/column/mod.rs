//! The column beside the chat, one per main window: the page the sidebar, a menu or a link
//! opened, and the pages opened over it inside the column, which Back returns from. The chat
//! stays beside it in its room (the room is the store's, shared by every window) and keeps its
//! place; closing the column gives the chat the whole width again.
//!
//! Every page shows here, by its [`Page`]: Needs you, Tasks, a task's page, Machines,
//! Settings, and any page added later. A page is added in three places: its variant at the end
//! of [`Page`], its width and title in [`frame`], and its view in the main window's
//! `page_view`. It is opened from anywhere in a main window with the window at hand:
//!
//! - [`open`]: afresh, the column holding it alone (the sidebar's rows, the menus, a link to a
//!   page);
//! - [`push`]: over what the column shows, so Back returns there (a task from the chat, a
//!   list or another task's page; alone when the column is shut).
//!
//! Back (⌘[) and Close (Esc) are the column's own ([`frame`]). The store hears what each
//! window's column holds ([`crate::store::Store::column_changed`]) and fetches for it: a
//! task's page, the machines, Settings' switches.

pub(crate) mod frame;
mod path;

use std::collections::HashMap;

use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Global, Window, WindowId};

pub(crate) use self::frame::{CloseColumn, GoBack, bind_keys};
use self::path::Path;
use crate::store::{Page, Store};

/// One main window's column.
pub(crate) struct Column {
    window: WindowId,
    store: Entity<Store>,
    path: Path,
}

impl Column {
    /// The column of `window`, shut, found by [`of`] from anywhere in that window until the
    /// window closes ([`closed`]).
    pub(crate) fn new(store: Entity<Store>, window: &Window, cx: &mut App) -> Entity<Self> {
        let id = window.window_handle().window_id();
        let column = cx.new(|_| Self {
            window: id,
            store,
            path: Path::default(),
        });
        cx.default_global::<Columns>().0.insert(id, column.clone());
        column
    }

    /// The page shown, if the column is open.
    #[must_use]
    pub(crate) fn current(&self) -> Option<Page> {
        self.path.current()
    }

    /// Whether Back returns somewhere besides shutting the column.
    #[must_use]
    pub(crate) fn can_go_back(&self) -> bool {
        self.path.can_go_back()
    }

    /// Opens `page` afresh, alone in the column.
    pub(crate) fn open(&mut self, page: Page, cx: &mut Context<'_, Self>) {
        if self.path.open(page) {
            self.changed(cx);
        }
    }

    /// Opens `page` over what the column shows (alone when it is shut); Back returns there.
    pub(crate) fn push(&mut self, page: Page, cx: &mut Context<'_, Self>) {
        if self.path.push(page) {
            self.changed(cx);
        }
    }

    /// Back: the page under the shown one, or the chat alone from the first.
    pub(crate) fn back(&mut self, cx: &mut Context<'_, Self>) {
        if self.path.back() {
            self.changed(cx);
        }
    }

    /// Shuts the column, whatever it shows.
    pub(crate) fn close(&mut self, cx: &mut Context<'_, Self>) {
        if self.path.close() {
            self.changed(cx);
        }
    }

    /// The path changed: the store hears it (it fetches what is shown now) and the window
    /// draws it.
    fn changed(&self, cx: &mut Context<'_, Self>) {
        let (window, pages) = (self.window, self.path.pages().to_vec());
        self.store
            .update(cx, |store, cx| store.column_changed(window, pages, cx));
        cx.notify();
    }
}

/// Every main window's column, by its window.
#[derive(Default)]
struct Columns(HashMap<WindowId, Entity<Column>>);

impl Global for Columns {}

/// `window`'s column, if it is a main window.
#[must_use]
pub(crate) fn of(window: &Window, cx: &App) -> Option<Entity<Column>> {
    cx.try_global::<Columns>()?
        .0
        .get(&window.window_handle().window_id())
        .cloned()
}

/// Opens `page` afresh in `window`'s column.
pub(crate) fn open(page: Page, window: &Window, cx: &mut App) {
    match of(window, cx) {
        Some(column) => column.update(cx, |column, cx| column.open(page, cx)),
        None => log::debug!("no column in this window for {page:?}"),
    }
}

/// Opens `page` over what `window`'s column shows; Back returns there.
pub(crate) fn push(page: Page, window: &Window, cx: &mut App) {
    match of(window, cx) {
        Some(column) => column.update(cx, |column, cx| column.push(page, cx)),
        None => log::debug!("no column in this window for {page:?}"),
    }
}

/// Opens `page` afresh in `window`'s column, or shuts the column if it shows it (a counter in
/// the window's bar).
pub(crate) fn toggle(page: Page, window: &Window, cx: &mut App) {
    if let Some(column) = of(window, cx) {
        column.update(cx, |column, cx| {
            if column.current() == Some(page) {
                column.close(cx);
            } else {
                column.open(page, cx);
            }
        });
    }
}

/// Whether `window`'s column shows `page`.
#[must_use]
pub(crate) fn shows(page: Page, window: &Window, cx: &App) -> bool {
    of(window, cx).is_some_and(|column| column.read(cx).current() == Some(page))
}

/// The main window `window` closed: its column is forgotten, here and by the store.
pub(crate) fn closed(window: WindowId, store: &Entity<Store>, cx: &mut App) {
    cx.default_global::<Columns>().0.remove(&window);
    store.update(cx, |store, cx| store.column_changed(window, Vec::new(), cx));
}
