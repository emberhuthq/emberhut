//! A room's catch-up: what came since the newest message held, asked for on its own (a few
//! messages, not the whole state) when the room comes on screen, so its new messages show
//! before its state does. On the reference server a room's state takes seconds to build; the
//! catch-up is a single query.

use std::sync::Arc;

use emberhut_api::ids::RoomId;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};

impl Store {
    /// Fetches what `room` got since the newest message held (nothing held: its newest page)
    /// and joins it to what is held. A failure only logs: the state poll brings the same.
    pub(super) fn catch_up(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        let after = self.chats.messages(room).last().map(|message| message.id);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { backend.catch_up(room, after) })
                .await;
            update_or_log(&this, cx, |store, cx| match page {
                Ok(page) => {
                    if store.chats.join_new(room, &page.chat) {
                        store.chat_changed(room, cx);
                    }
                }
                Err(error) => log::debug!("couldn't catch up on room {room}: {error}"),
            });
        })
        .detach();
    }
}
