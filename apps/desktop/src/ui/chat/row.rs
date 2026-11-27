//! What a message's row takes from the chat view: its text as read once (the view keeps each
//! message read, [`ParsedCache`], and reads one again only when its text changes), how wide its
//! text may be, whether it is lit or shows whole, and what its Reply, Copy, Show all and quote
//! do.
//!
//! [`ParsedCache`]: crate::prose::parsed::ParsedCache

use std::rc::Rc;

use emberhut_api::chat::ChatMessage;
use emberhut_api::ids::MessageId;
use gpui_kit::prelude::*;
use gpui_kit::{App, ListState, Pixels, WeakEntity, canvas};

use super::ChatView;
use crate::prose::parsed::Parsed;
use crate::ui::message::RowState;
use crate::ui::prose::Place;

/// How wide an answer's text may be before the list has a width.
pub(super) const TEXT_WIDTH_AT_FIRST: f32 = 600.;

/// How wide an answer's text may be in a list `list` pixels wide: 82% of the column (at most
/// 880 pixels, less its padding), less the bubble's own.
pub(super) fn text_width(list: f32) -> f32 {
    if list <= 0. {
        return TEXT_WIDTH_AT_FIRST;
    }
    (list.min(880.) - 56.).max(200.) * 0.82 - 28.
}

/// Asks for the next frame when the list was laid out at another width than `drawn_for`, the
/// width its rows were drawn for: a row's text width is the list's from its last layout (the
/// list can't be asked while it lays its rows out), so after the chat's width changes (the
/// column beside it opened or closed, the window resized) the rows are drawn again at once at
/// the width the list has now, not whenever something else redraws the chat.
pub(super) fn follow_width(list: ListState, drawn_for: Pixels) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, (), window, _| {
            if list.viewport_bounds().size.width != drawn_for {
                window.request_animation_frame();
            }
        },
    )
    .absolute()
    .size_0()
}

/// Runs `act` on the chat view, if it is still there.
fn on_view(
    view: &WeakEntity<ChatView>,
    cx: &mut App,
    act: impl FnOnce(&mut ChatView, &mut Context<'_, ChatView>),
) {
    if let Err(error) = view.update(cx, act) {
        log::debug!("the chat is gone: {error}");
    }
}

impl ChatView {
    /// `messages`, each as it was read (read now if it wasn't, or its text changed).
    pub(super) fn parsed(&mut self, messages: &[ChatMessage]) -> Vec<Rc<Parsed>> {
        messages
            .iter()
            .map(|message| self.parsed.get(message))
            .collect()
    }

    /// What the row of `message` needs from the view.
    pub(super) fn row_state(&mut self, message: &ChatMessage, cx: &Context<'_, Self>) -> RowState {
        let id = message.id;
        let view = cx.entity().downgrade();
        let (reply_view, copy_view, all_view) = (view.clone(), view.clone(), view.clone());
        RowState {
            parsed: self.parsed.get(message),
            lit: self.lit == Some(id),
            now: self.now.clone(),
            place: Place {
                message: id,
                width: self.text_width,
                copied: self
                    .copied
                    .filter(|&(message, _)| message == id)
                    .map(|(_, index)| index),
                on_copy: Rc::new(move |index, _, cx| {
                    on_view(&copy_view, cx, |this, cx| this.copy_quote(id, index, cx));
                }),
                whole: self.whole.contains(&id),
                on_show_all: Rc::new(move |_, cx| {
                    on_view(&all_view, cx, |this, cx| this.show_whole(id, cx));
                }),
            },
            on_reply: Rc::new(move |window, cx| {
                on_view(&reply_view, cx, |this, cx| this.reply_to(id, window, cx));
            }),
            on_jump: Rc::new(move |quoted, _, cx| {
                on_view(&view, cx, |this, cx| this.jump_to(quoted, cx));
            }),
        }
    }

    /// "Show all" on message `id`: all of its text shows, as plain text.
    fn show_whole(&mut self, id: MessageId, cx: &mut Context<'_, Self>) {
        if self.whole.insert(id) {
            self.list.remeasure();
            cx.notify();
        }
    }

    /// Forgets what was read of messages the room no longer holds.
    pub(super) fn forget_parsed(&mut self, cx: &Context<'_, Self>) {
        let store = self.store.read(cx);
        let held: std::collections::HashSet<MessageId> = store
            .chats()
            .messages(store.room())
            .iter()
            .map(|message| message.id)
            .collect();
        self.parsed.keep(|id| held.contains(&id));
        self.whole.retain(|id| held.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;
    use serde_json::json;

    use super::super::tests::open_chat;
    use super::*;
    use crate::fixtures::{message_at, room, state};
    use crate::prose::blocks::MOST_TEXT;

    #[test]
    fn the_text_width_follows_the_list() {
        assert!((text_width(0.) - TEXT_WIDTH_AT_FIRST).abs() < f32::EPSILON);
        assert!(
            text_width(2000.) > text_width(600.),
            "up to the column's width"
        );
        assert!((text_width(2000.) - text_width(5000.)).abs() < f32::EPSILON);
    }

    #[gpui_kit::test]
    fn a_message_is_read_once_however_often_the_chat_is_drawn(cx: &mut TestAppContext) {
        let (_backend, store, window) = open_chat(cx);
        let answer = message_at(
            1,
            "2026-11-25T08:00:00Z",
            json!({"text": "The **plan** is ready."}),
        );
        store.update(cx, |store, cx| {
            store.take_state_for_tests(
                state(json!({"rooms": [room(1, json!({}))], "chat": [answer]})),
                cx,
            );
        });
        cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        let first = window
            .update(cx, |view, _, _| {
                view.parsed.read(MessageId::new(1)).cloned()
            })
            .unwrap()
            .expect("read when drawn");
        for _ in 0..3 {
            window.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        window
            .update(cx, |view, _, _| {
                let now = view.parsed.read(MessageId::new(1)).unwrap();
                assert!(Rc::ptr_eq(&first, now), "not read again");
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn show_all_shows_a_long_message_whole(cx: &mut TestAppContext) {
        let (_backend, store, window) = open_chat(cx);
        let long = "A line.\n".repeat(MOST_TEXT / 4);
        let answer = message_at(1, "2026-11-25T08:00:00Z", json!({ "text": long }));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(
                state(json!({"rooms": [room(1, json!({}))], "chat": [answer]})),
                cx,
            );
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("show-all-1", cx);
        })
        .unwrap();
        window
            .update(cx, |view, _, _| {
                assert!(view.whole.contains(&MessageId::new(1)));
            })
            .unwrap();
    }
}
