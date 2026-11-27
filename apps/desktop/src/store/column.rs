//! The room the chat shows (every main window's, as the store is) and what the windows'
//! columns hold, as the store needs it. Each main window keeps its own column (its page and the
//! ones under it, `ui::column`) and tells the store whenever it changes, through
//! [`Store::column_changed`]: what a column shows now is fetched (a task's page, the machines,
//! Settings' switches), and a task's page off every column is forgotten.

use std::collections::HashMap;

use emberhut_api::ids::{RoomId, TaskId};
use gpui_kit::{Context, WindowId};

use super::{Page, Store};

/// What each main window's column holds, the shown page last; a shut column isn't here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Columns(HashMap<WindowId, Vec<Page>>);

impl Columns {
    /// The pages the columns show, one per open column.
    fn shown(&self) -> impl Iterator<Item = Page> + '_ {
        self.0.values().filter_map(|pages| pages.last().copied())
    }

    /// Whether a column shows `page`.
    #[must_use]
    pub(crate) fn shows(&self, page: Page) -> bool {
        self.shown().any(|shown| shown == page)
    }

    /// Whether a column shows a page `test` picks.
    #[must_use]
    pub(crate) fn shows_any(&self, test: impl Fn(Page) -> bool) -> bool {
        self.shown().any(test)
    }

    /// The tasks whose pages the columns show.
    #[must_use]
    pub(crate) fn shown_tasks(&self) -> Vec<TaskId> {
        let mut tasks: Vec<TaskId> = self
            .shown()
            .filter_map(|page| match page {
                Page::Task(task) => Some(task),
                _ => None,
            })
            .collect();
        tasks.sort_unstable();
        tasks.dedup();
        tasks
    }

    /// Whether `task`'s page is in a column, shown or under another.
    #[must_use]
    pub(crate) fn holds_task(&self, task: TaskId) -> bool {
        self.0
            .values()
            .flatten()
            .any(|page| *page == Page::Task(task))
    }

    /// Every page in a column, shown or under another (a page in two columns, twice).
    pub(crate) fn held(&self) -> impl Iterator<Item = Page> + '_ {
        self.0.values().flatten().copied()
    }

    /// Whether `page` is in a column, shown or under another.
    #[must_use]
    pub(crate) fn holds(&self, page: Page) -> bool {
        self.0.values().flatten().any(|held| *held == page)
    }

    /// `window`'s column holds `pages` now; the page it showed before.
    pub(super) fn set(&mut self, window: WindowId, pages: Vec<Page>) -> Option<Page> {
        let before = if pages.is_empty() {
            self.0.remove(&window)
        } else {
            self.0.insert(window, pages)
        };
        before.and_then(|pages| pages.last().copied())
    }
}

impl Store {
    /// Whether a window's column shows `page`.
    #[must_use]
    pub(crate) fn shows_page(&self, page: Page) -> bool {
        self.columns.shows(page)
    }

    /// `window`'s column holds `pages` now (none: shut, or the window closed). A page it shows
    /// afresh is fetched: the machines at once, Settings' switches or what one of its pages
    /// shows, a task's page.
    pub(crate) fn column_changed(
        &mut self,
        window: WindowId,
        pages: Vec<Page>,
        cx: &mut Context<'_, Self>,
    ) {
        let now = pages.last().copied();
        let before = self.columns.set(window, pages);
        if now != before {
            match now {
                // Their loop sleeps off their page: it fetches them now (if the window is in
                // front).
                Some(Page::Machines) => self.machines.kick(),
                Some(Page::Settings) => self.settings_opened(cx),
                Some(page @ (Page::Project(_) | Page::Person(_))) => self.place_shown(page, cx),
                Some(Page::Scheduled) => self.reload_scheduled(),
                Some(Page::SettingsPane(pane)) => self.settings_pane_shown(pane, cx),
                _ => {}
            }
            self.working_page_shown(now);
            self.approvals_page_shown(now, cx);
        }
        self.forget_closed_jobs();
        self.forget_closed_approvals();
        self.tasks_on_screen(cx);
        cx.notify();
    }

    /// Opens `page` alone in one window's column, shuts it (`None`), for the tests.
    #[cfg(test)]
    pub(crate) fn show_in_test_column(&mut self, page: Option<Page>, cx: &mut Context<'_, Self>) {
        let pages = page.into_iter().collect();
        self.column_changed(WindowId::from(1), pages, cx);
    }

    /// Opens `room`'s chat in every main window, beside whatever their columns show (a page
    /// stays open, as in the reference client's sidebar).
    pub(crate) fn switch_room(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        self.enter_room(room, cx);
        // Back on a room whose state is on hand: its unread answers are read now.
        self.mark_read(cx);
        cx.notify();
    }

    /// Makes `room` the chat's room; a room other than the open one starts fresh (its state,
    /// its live stream, its pictures).
    pub(super) fn enter_room(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        if room == self.room {
            return;
        }
        self.room = room;
        self.etag = None;
        self.live = None;
        self.pending = None;
        self.live_feed.follow(room);
        self.keep_pictures_for(room, cx);
        self.switch_in(room, cx);
        self.room_opened(cx);
        self.kicker.kick();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, Entity, TestAppContext};

    use super::*;
    use crate::backend::Backend;
    use crate::backend::fake::FakeBackend;

    const OTHER: RoomId = RoomId::new(2);
    const ONE: TaskId = TaskId::new(1);
    const TWO: TaskId = TaskId::new(2);

    fn store(cx: &mut TestAppContext) -> Entity<Store> {
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn Backend>;
        cx.new(|_| Store::for_tests(backend))
    }

    fn window(n: u64) -> WindowId {
        WindowId::from(n)
    }

    #[gpui_kit::test]
    fn another_room_leaves_the_columns_as_they_are(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            store.column_changed(window(1), vec![Page::Tasks], cx);
            store.switch_room(OTHER, cx);
        });
        store.read_with(cx, |store, _| {
            assert_eq!(store.room(), OTHER);
            assert!(store.shows_page(Page::Tasks));
        });
    }

    #[gpui_kit::test]
    fn the_store_hears_every_windows_column(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            store.column_changed(window(1), vec![Page::Needs, Page::Task(ONE)], cx);
            store.column_changed(window(2), vec![Page::Task(TWO), Page::Machines], cx);
        });
        store.read_with(cx, |store, _| {
            let columns = &store.columns;
            assert!(columns.shows(Page::Task(ONE)));
            assert!(columns.shows(Page::Machines));
            assert!(!columns.shows(Page::Needs), "under a task");
            assert_eq!(columns.shown_tasks(), [ONE]);
            assert!(columns.holds_task(TWO), "under the machines");
        });
        store.update(cx, |store, cx| {
            store.column_changed(window(2), Vec::new(), cx);
        });
        store.read_with(cx, |store, _| {
            assert!(!store.columns.holds_task(TWO), "its window's column shut");
            assert!(store.columns.holds_task(ONE));
        });
    }
}
