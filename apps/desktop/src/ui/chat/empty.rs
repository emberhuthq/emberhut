//! What a room's chat shows with no messages: the welcome and its starters once the chat is
//! known to be empty, nothing while a message sent from here waits under it (the footer shows
//! that one), and otherwise why there is nothing yet (it is loading, or the server's problem).

use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, div};

use super::{ChatView, reading};
use crate::store::Store;
use crate::ui::palette::Pal;

/// What an empty chat shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EmptyChat {
    /// The welcome, with its starters.
    Welcome,
    /// Nothing: a message sent from here shows in the footer.
    Blank,
    /// A line on why there is nothing yet.
    Note(String),
}

impl EmptyChat {
    /// What the open room's chat shows while it has no messages.
    #[must_use]
    pub(super) fn of(store: &Store) -> Self {
        if let Some(problem) = store.problem() {
            return Self::Note(problem.to_owned());
        }
        if !store.is_state_here() || !store.chats().is_held(store.room()) {
            return Self::Note("Loading the chat…".to_owned());
        }
        if store.pending().is_some() {
            Self::Blank
        } else {
            Self::Welcome
        }
    }
}

impl ChatView {
    /// The body of a chat with no messages (see [`EmptyChat`]).
    pub(super) fn empty_body(&self, p: &Pal, cx: &mut Context<'_, Self>) -> AnyElement {
        match EmptyChat::of(self.store.read(cx)) {
            EmptyChat::Welcome => {
                let store = self.store.clone();
                reading::welcome(p, move |starter, _, cx| {
                    store.update(cx, |store, cx| store.start_message(starter.to_string(), cx));
                })
                .into_any_element()
            }
            EmptyChat::Blank => div().flex_1().into_any_element(),
            EmptyChat::Note(line) => div()
                .id("empty-chat")
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.muted)
                .child(line)
                .test_support()
                .into_any_element(),
        }
    }
}
