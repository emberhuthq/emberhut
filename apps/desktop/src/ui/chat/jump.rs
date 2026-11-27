//! What the chat does for one message: Reply quotes it above the composer; a click on a quote
//! goes back to the message quoted (loading earlier pages of the same room, up to ten, until
//! it is held; leaving the room ends it) and lights it for a moment; Copy on a quote copies
//! its words and says "Copied" for a moment.

use std::time::Duration;

use emberhut_api::ids::{MessageId, RoomId};
use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, ListOffset, Window, px};

use super::ChatView;
use crate::store::Store;
use crate::ui::message::Item;

/// The most earlier pages a jump loads looking for the message quoted.
const MOST_PAGES: u8 = 10;

/// How long the message jumped to stays lit.
const LIT_FOR: Duration = Duration::from_millis(1600);

/// How long a quote's Copy says "Copied".
const COPIED_FOR: Duration = Duration::from_secs(2);

/// A jump under way: the room it is in, the message it looks for, and how many earlier pages
/// it has loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Jump {
    room: RoomId,
    to: MessageId,
    pages: u8,
}

impl ChatView {
    /// Reply on message `id`: it is quoted above the composer, which takes the keyboard.
    pub(super) fn reply_to(
        &mut self,
        id: MessageId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let store = self.store.read(cx);
        let Some(message) = store
            .chats()
            .messages(store.room())
            .iter()
            .find(|message| message.id == id)
            .cloned()
        else {
            return;
        };
        self.store
            .update(cx, |store, cx| store.reply_to(&message, cx));
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// A click on a quote of message `id`: there, once it is held.
    pub(super) fn jump_to(&mut self, id: MessageId, cx: &mut Context<'_, Self>) {
        let room = self.store.read(cx).room();
        self.jump = Some(Jump {
            room,
            to: id,
            pages: 0,
        });
        self.continue_jump(cx);
    }

    /// Goes on with the jump under way: to the message once it is held, else the next earlier
    /// page (one at a time), until there are none or [`MOST_PAGES`] came without it. Another
    /// room open ends it.
    pub(super) fn continue_jump(&mut self, cx: &mut Context<'_, Self>) {
        let Some(jump) = self.jump else { return };
        let store = self.store.read(cx);
        let room = store.room();
        if room != jump.room {
            self.jump = None;
            return;
        }
        let messages = store.chats().messages(room);
        if let Some(at) = messages.iter().position(|message| message.id == jump.to) {
            // A folded event line shows in its run's first row.
            let row = (0..=at)
                .rev()
                .find(|&row| Item::at(messages, row, &self.now) != Item::Folded)
                .unwrap_or(at);
            let lit = messages.get(row).map_or(jump.to, |message| message.id);
            self.jump = None;
            // Row 0 is "Show earlier"; a little above the message, so it isn't under the
            // header.
            self.list.scroll_to(ListOffset {
                item_ix: row + 1,
                offset_in_item: px(-80.),
            });
            self.light(lit, cx);
            return;
        }
        if store.chats().is_loading_earlier(room) {
            return;
        }
        let earlier = store.state().is_some_and(|state| state.earlier);
        if !store.chats().has_earlier(room, earlier) || jump.pages >= MOST_PAGES {
            log::info!("the message quoted ({}) isn't in the chat held", jump.to);
            self.jump = None;
            return;
        }
        self.jump = Some(Jump {
            pages: jump.pages + 1,
            ..jump
        });
        self.store.update(cx, Store::load_earlier);
    }

    /// Lights message `id` for [`LIT_FOR`].
    fn light(&mut self, id: MessageId, cx: &mut Context<'_, Self>) {
        self.lit = Some(id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LIT_FOR).await;
            let dimmed = this.update(cx, |this, cx| {
                if this.lit == Some(id) {
                    this.lit = None;
                    cx.notify();
                }
            });
            if let Err(error) = dimmed {
                log::debug!("the chat is gone: {error}");
            }
        })
        .detach();
    }

    /// Copy on quote `index` of message `id`: its words on the clipboard, "Copied" for a moment.
    pub(super) fn copy_quote(&mut self, id: MessageId, index: usize, cx: &mut Context<'_, Self>) {
        let Some(words) = self
            .parsed
            .read(id)
            .and_then(|parsed| parsed.quote_words(index))
            .map(str::to_owned)
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(words));
        let copied = Some((id, index));
        self.copied = copied;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let reset = this.update(cx, |this, cx| {
                if this.copied == copied {
                    this.copied = None;
                    cx.notify();
                }
            });
            if let Err(error) = reset {
                log::debug!("the chat is gone: {error}");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::chat::{ChatMessage, ChatPage, PAGE_SIZE};
    use emberhut_api::ids::TaskId;
    use emberhut_api::talk::TalkRequest;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Entity, TestAppContext, WindowHandle};
    use serde_json::json;

    use super::super::tests::{open_chat, open_chat_over};
    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::fixtures::{message_at, room, state};
    use crate::store::Page;
    use crate::wire::TASK_KEY_PREFIX;

    fn chat() -> emberhut_api::state::State {
        let first = message_at(
            1,
            "2026-11-25T08:00:00Z",
            json!({"text": "The plan is ready.",
                "refs": [{"kind": "task", "label": format!("{TASK_KEY_PREFIX}7"), "task": 7,
                    "title": "The plan"}]}),
        );
        let reply = message_at(
            2,
            "2026-11-25T08:01:00Z",
            json!({"mine": true, "text": "Thanks",
                "reply": {"id": 1, "mine": false, "at": "2026-11-25T08:00:00Z",
                    "text": "The plan is ready.", "file": null}}),
        );
        state(json!({"rooms": [room(1, json!({}))], "chat": [first, reply]}))
    }

    #[gpui_kit::test]
    fn a_reply_carries_the_message_it_quotes(cx: &mut TestAppContext) {
        let (backend, store, window) = open_chat(cx);
        store.update(cx, |store, cx| store.take_state_for_tests(chat(), cx));
        window
            .update(cx, |view, window, cx| {
                view.reply_to(MessageId::new(1), window, cx);
                view.input
                    .update(cx, |input, cx| input.set_value("Go ahead", window, cx));
            })
            .unwrap();
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.replying().map(|quote| quote.id),
                Some(MessageId::new(1))
            );
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("send", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let mut request = TalkRequest::text(RoomId::MAIN, "Go ahead");
        request.reply_to = Some(MessageId::new(1));
        assert_eq!(backend.calls().last(), Some(&Call::Talk(request)));
        store.read_with(cx, |store, _| assert_eq!(store.replying(), None));
    }

    #[gpui_kit::test]
    fn a_click_on_a_quote_lights_the_original(cx: &mut TestAppContext) {
        let (_backend, store, window) = open_chat(cx);
        store.update(cx, |store, cx| store.take_state_for_tests(chat(), cx));
        window
            .update(cx, |view, _, cx| {
                view.jump_to(MessageId::new(1), cx);
                assert_eq!(view.lit, Some(MessageId::new(1)));
                assert_eq!(view.jump, None);
            })
            .unwrap();
        cx.executor().advance_clock(LIT_FOR * 2);
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                assert_eq!(view.lit, None, "only for a moment");
            })
            .unwrap();
    }

    /// A chat holding messages 500 and 501 (which quotes message 1), with earlier ones on the
    /// server: a full page of 400 to 459, then one with 1.
    fn paging(
        cx: &mut TestAppContext,
    ) -> (Arc<FakeBackend>, Entity<Store>, WindowHandle<ChatView>) {
        let backend = FakeBackend::new([]);
        let full: Vec<ChatMessage> = (400..400 + i64::try_from(PAGE_SIZE).unwrap())
            .map(|id| message_at(id, "2026-11-24T08:00:00Z", json!({})))
            .collect();
        backend.push_history(Ok(ChatPage { chat: full }));
        backend.push_history(Ok(ChatPage {
            chat: vec![message_at(1, "2026-11-23T08:00:00Z", json!({}))],
        }));
        let (backend, store, window) = open_chat_over(backend, cx);
        let newest = [
            message_at(500, "2026-11-25T08:00:00Z", json!({})),
            message_at(
                501,
                "2026-11-25T08:01:00Z",
                json!({"mine": true, "text": "This one",
                    "reply": {"id": 1, "mine": false, "at": "2026-11-23T08:00:00Z",
                        "text": "m1", "file": null}}),
            ),
        ];
        store.update(cx, |store, cx| {
            store.take_state_for_tests(
                state(json!({"rooms": [room(1, json!({})), room(2, json!({}))],
                    "chat": newest, "earlier": true})),
                cx,
            );
        });
        (backend, store, window)
    }

    fn pages_asked(backend: &FakeBackend) -> usize {
        backend
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::History(..)))
            .count()
    }

    #[gpui_kit::test]
    fn a_jump_loads_earlier_pages_until_the_quoted_message_comes(cx: &mut TestAppContext) {
        let (backend, _store, window) = paging(cx);
        window
            .update(cx, |view, _, cx| view.jump_to(MessageId::new(1), cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            pages_asked(&backend),
            2,
            "a full page, then the one with it"
        );
        window
            .update(cx, |view, _, _| {
                assert_eq!(view.lit, Some(MessageId::new(1)));
                assert_eq!(view.jump, None);
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn leaving_the_room_ends_a_jump(cx: &mut TestAppContext) {
        let (backend, store, window) = paging(cx);
        window
            .update(cx, |view, _, cx| view.jump_to(MessageId::new(1), cx))
            .unwrap();
        store.update(cx, |store, cx| store.switch_room(RoomId::new(2), cx));
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| assert_eq!(view.jump, None))
            .unwrap();
        assert_eq!(
            pages_asked(&backend),
            1,
            "only the page asked before leaving"
        );
    }

    #[gpui_kit::test]
    fn a_tasks_chip_opens_its_page(cx: &mut TestAppContext) {
        let (_backend, store, window) = open_chat(cx);
        store.update(cx, |store, cx| store.take_state_for_tests(chat(), cx));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("ref-1-0", cx);
        })
        .unwrap();
        store.read_with(cx, |store, _| {
            assert!(
                store.shows_page(Page::Task(TaskId::new(7))),
                "its page in the window's column"
            );
        });
    }
}
