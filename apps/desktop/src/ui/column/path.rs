//! A column's pages: the one opened afresh (from the sidebar, a menu, a link to a page) first,
//! the ones opened over it after, the shown one last. Back takes the shown one off; from the
//! first, the column shuts.

use crate::store::Page;

/// The pages in one window's column, the shown one last; empty while the column is shut.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Path(Vec<Page>);

impl Path {
    /// The page shown, if the column is open.
    #[must_use]
    pub(crate) fn current(&self) -> Option<Page> {
        self.0.last().copied()
    }

    /// Every page on the path, the first opened first.
    #[must_use]
    pub(crate) fn pages(&self) -> &[Page] {
        &self.0
    }

    /// Whether Back has somewhere to go besides shutting the column: a page under the shown
    /// one, or (a task opened alone, from the chat) the chat itself.
    #[must_use]
    pub(crate) fn can_go_back(&self) -> bool {
        match self.0.as_slice() {
            [] => false,
            [only] => matches!(only, Page::Task(_)),
            _ => true,
        }
    }

    /// Opens `page` afresh: whatever was on the path goes. A room isn't a page of the column
    /// (it is the chat's): nothing changes. Whether the path changed.
    pub(crate) fn open(&mut self, page: Page) -> bool {
        if matches!(page, Page::Room(_)) || self.0 == [page] {
            return false;
        }
        self.0.clear();
        self.0.push(page);
        true
    }

    /// Opens `page` over the shown one, so Back returns there (the same page twice in a row
    /// is one); with the column shut it opens alone. Whether the path changed.
    pub(crate) fn push(&mut self, page: Page) -> bool {
        if matches!(page, Page::Room(_)) || self.current() == Some(page) {
            return false;
        }
        self.0.push(page);
        true
    }

    /// Back: the page under the shown one, or shut from the first. Whether the path changed.
    pub(crate) fn back(&mut self) -> bool {
        self.0.pop().is_some()
    }

    /// Shuts the column, whatever it shows. Whether the path changed.
    pub(crate) fn close(&mut self) -> bool {
        let open = !self.0.is_empty();
        self.0.clear();
        open
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::ids::{RoomId, TaskId};

    use super::*;

    const ONE: Page = Page::Task(TaskId::new(1));
    const TWO: Page = Page::Task(TaskId::new(2));

    #[test]
    fn back_returns_through_the_pages_opened_over_each_other() {
        let mut path = Path::default();
        assert!(path.open(Page::Needs));
        assert!(path.push(Page::Tasks));
        assert!(path.push(ONE));
        assert!(!path.push(ONE), "the same page twice in a row is one");
        assert_eq!(path.pages(), [Page::Needs, Page::Tasks, ONE]);
        assert!(path.can_go_back());
        assert!(path.back());
        assert_eq!(path.current(), Some(Page::Tasks));
        assert!(path.back());
        assert_eq!(path.current(), Some(Page::Needs));
        assert!(!path.can_go_back(), "the first page: only Close");
        assert!(path.back(), "Back from the first shuts the column");
        assert_eq!(path.current(), None);
        assert!(!path.back(), "nothing more");
    }

    #[test]
    fn a_page_opened_afresh_forgets_the_ones_under_and_close_shuts_them_all() {
        let mut path = Path::default();
        path.open(Page::Needs);
        path.push(ONE);
        assert!(path.open(Page::Settings));
        assert_eq!(path.pages(), [Page::Settings]);
        assert!(!path.open(Page::Settings), "already so");
        path.push(TWO);
        assert!(path.close());
        assert_eq!(path.current(), None);
        assert!(!path.close());
    }

    #[test]
    fn a_task_opened_with_the_column_shut_is_alone_and_back_goes_to_the_chat() {
        let mut path = Path::default();
        assert!(path.push(ONE));
        assert_eq!(path.pages(), [ONE]);
        assert!(path.can_go_back(), "Back to the chat");
        path.push(TWO);
        assert_eq!(path.pages(), [ONE, TWO]);
    }

    #[test]
    fn a_room_is_the_chats_not_the_columns() {
        let mut path = Path::default();
        assert!(!path.open(Page::Room(RoomId::MAIN)));
        assert!(!path.push(Page::Room(RoomId::MAIN)));
        assert_eq!(path.current(), None);
    }
}
