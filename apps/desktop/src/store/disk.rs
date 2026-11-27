//! The store's side of what is kept on disk ([`DiskCache`]): what a launch starts from, saving a
//! room's chat whenever it changes, and clearing it all when this device is signed out or
//! refused.

use std::sync::Arc;

use emberhut_api::chat::ChatMessage;
use emberhut_api::ids::RoomId;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::Store;
use super::on_disk::DiskCache;

/// What a launch starts from, read before the app's run loop starts (no file is read on the UI
/// thread): the cache, and the open room's chat from it.
#[derive(Debug, Default)]
pub(crate) struct Kept {
    /// The cache (none without a cache directory, or in the tests).
    pub(crate) disk: Option<Arc<DiskCache>>,
    /// The open room's newest messages.
    pub(crate) chat: Option<Vec<ChatMessage>>,
}

impl Kept {
    /// What `disk` holds for `room`.
    #[must_use]
    pub(crate) fn read(disk: Option<DiskCache>, room: RoomId) -> Self {
        let Some(disk) = disk else {
            return Self::default();
        };
        Self {
            chat: disk.chat(room),
            disk: Some(Arc::new(disk)),
        }
    }
}

impl Store {
    /// Starts from what was kept: the open room's chat is held, to show with the server's first
    /// state. Nothing shows before that state: the server may no longer let this device in
    /// ([`Gate`](super::gate::Gate)).
    pub(crate) fn start_from(&mut self, kept: Kept, cx: &mut Context<'_, Self>) {
        let Kept { disk, chat } = kept;
        self.disk = disk;
        if let Some(chat) = chat {
            self.chats.restore(self.room, chat);
        }
        self.fetch_pictures(self.room, cx);
        // What came since it was kept, before the state's few seconds.
        self.catch_up(self.room, cx);
        cx.notify();
    }

    /// `room`'s held chat changed: its pictures are fetched, it is saved, and the views
    /// redraw.
    pub(super) fn chat_changed(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        self.fetch_pictures(room, cx);
        if let Some(disk) = self.disk.clone() {
            // Only the copy is made here; the encoding and the write are the background's.
            let save = disk.save_chat(room, self.chats.messages(room));
            cx.background_spawn(async move { disk.write(&save) })
                .detach();
        }
        cx.notify();
    }

    /// Removes the chats kept for this server, off the UI thread: this device was signed out
    /// or refused.
    pub(super) fn clear_kept(&self, cx: &mut Context<'_, Self>) {
        if let Some(disk) = self.disk.clone() {
            cx.background_spawn(async move { disk.clear() }).detach();
        }
    }

    /// Reads `room`'s kept chat off the UI thread when none of it is held yet, then catches up
    /// on what came since: a room switched to shows its chat at once, then its new messages,
    /// then its state.
    pub(super) fn switch_in(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        let disk = self.disk.clone().filter(|_| !self.chats.is_held(room));
        let Some(disk) = disk else {
            self.catch_up(room, cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            let saved = cx.background_spawn(async move { disk.chat(room) }).await;
            super::update_or_log(&this, cx, |store, cx| {
                if let Some(saved) = saved {
                    store.chats.restore(room, saved);
                    store.fetch_pictures(room, cx);
                    cx.notify();
                }
                store.catch_up(room, cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use emberhut_api::chat::ChatPage;
    use emberhut_api::headers::ETag;
    use emberhut_api::ids::MessageId;
    use gpui_kit::{Entity, TestAppContext};
    use serde_json::json;

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::backend::{FreshState, StateReply};
    use crate::fixtures::{message, state};
    use crate::store::ThemeState;
    use crate::store::gate::Gate;

    const OTHER: RoomId = RoomId::new(2);

    fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
        let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
    }

    fn ids(store: &Store, room: RoomId) -> Vec<i64> {
        store
            .chats
            .messages(room)
            .iter()
            .map(|m| m.id.get())
            .collect()
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "it stands for a poll's reply, which is a Result"
    )]
    fn fresh(chat: &[i64]) -> Result<StateReply, crate::backend::Error> {
        let chat: Vec<_> = chat.iter().copied().map(message).collect();
        Ok(StateReply::Fresh(FreshState {
            state: Box::new(state(json!({ "chat": chat }))),
            etag: Some(ETag::new("\"v1\"")),
        }))
    }

    #[gpui_kit::test]
    fn a_launch_holds_the_kept_chat_and_shows_it_once_the_server_lets_it_in(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskCache::in_dir(dir.path());
        disk.write(&disk.save_chat(RoomId::MAIN, &[message(1), message(2)]));
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.start_from(Kept::read(Some(disk), RoomId::MAIN), cx);
            assert_eq!(ids(store, RoomId::MAIN), [1, 2], "held");
            assert_eq!(store.gate(), Gate::Connecting, "not shown yet");
            store.apply(RoomId::MAIN, fresh(&[2, 3]), Instant::now(), cx);
            assert_eq!(store.gate(), Gate::Open);
            assert_eq!(ids(store, RoomId::MAIN), [1, 2, 3]);
        });
    }

    #[gpui_kit::test]
    fn a_changed_chat_is_kept_for_the_next_launch_and_an_unchanged_one_isnt_written_again(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("chat-1.json");
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.start_from(
                Kept::read(Some(DiskCache::in_dir(dir.path())), RoomId::MAIN),
                cx,
            );
            store.apply(RoomId::MAIN, fresh(&[7]), Instant::now(), cx);
        });
        cx.run_until_parked();
        let kept = DiskCache::in_dir(dir.path());
        assert_eq!(kept.chat(RoomId::MAIN).map(|chat| chat.len()), Some(1));
        std::fs::remove_file(&file).unwrap();
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, fresh(&[7]), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert!(!file.exists(), "the same chat: nothing written");
    }

    #[gpui_kit::test]
    fn a_refused_device_has_its_kept_chats_removed(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("server");
        let disk = DiskCache::in_dir(&folder);
        disk.write(&disk.save_chat(RoomId::MAIN, &[message(1)]));
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.start_from(Kept::read(Some(disk), RoomId::MAIN), cx);
            let refused = Err(crate::backend::Error::Refused {
                status: Some(401),
                message: None,
            });
            store.apply(RoomId::MAIN, refused, Instant::now(), cx);
        });
        cx.run_until_parked();
        assert!(!folder.exists());
    }

    #[gpui_kit::test]
    fn a_room_switched_to_shows_its_kept_chat_then_catches_up(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskCache::in_dir(dir.path());
        disk.write(&disk.save_chat(OTHER, &[message(4), message(5)]));
        let backend = Arc::new(FakeBackend::new([]));
        // The launch's catch-up for Main, then room 2's.
        backend.push_catch_up(Ok(ChatPage::default()));
        backend.push_catch_up(Ok(ChatPage {
            chat: vec![message(6)],
        }));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.start_from(Kept::read(Some(disk), RoomId::MAIN), cx);
            store.switch_room(OTHER, cx);
        });
        cx.run_until_parked();
        assert!(
            backend
                .calls()
                .contains(&Call::CatchUp(OTHER, Some(MessageId::new(5)))),
            "only what came after the newest kept: {:?}",
            backend.calls()
        );
        store.read_with(cx, |store, _| assert_eq!(ids(store, OTHER), [4, 5, 6]));
    }

    #[gpui_kit::test]
    fn a_room_with_nothing_kept_catches_up_on_its_newest_page(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| store.switch_room(OTHER, cx));
        cx.run_until_parked();
        assert!(backend.calls().contains(&Call::CatchUp(OTHER, None)));
    }
}
