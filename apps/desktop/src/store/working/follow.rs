//! The loop behind a page that refreshes while it shows: it fetches when the page is wanted,
//! then waits its period (or a kick) while it still is, and only a kick while it isn't. What
//! makes it wanted again (its page opening, the window coming to the front) kicks it.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::backend::Backend;
use crate::store::poll::Kicked;
use crate::store::{Store, update_or_log};

/// What a followed feed does.
pub(in crate::store) struct Feed<T> {
    /// How long it waits between fetches while wanted.
    pub(in crate::store) every: Duration,
    /// Whether to fetch now.
    pub(in crate::store) wanted: fn(&Store) -> bool,
    /// The fetch, on a background thread.
    pub(in crate::store) fetch: fn(&dyn Backend) -> T,
    /// Takes a fetch's outcome in; whether anything changed.
    pub(in crate::store) take: fn(&mut Store, T) -> bool,
}

impl<T> Clone for Feed<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Feed<T> {}

/// Runs `feed`'s loop for the store in `cx`, woken by `kicked`.
pub(in crate::store) fn follow<T: Send + 'static>(
    feed: Feed<T>,
    kicked: Kicked,
    cx: &mut Context<'_, Store>,
) {
    cx.spawn(async move |this, cx| {
        loop {
            let Some(due) = update_or_log(&this, cx, |store, _| {
                (feed.wanted)(store).then(|| Arc::clone(&store.backend))
            }) else {
                return;
            };
            if let Some(backend) = due {
                let fetch = feed.fetch;
                let got = cx
                    .background_spawn(async move { fetch(backend.as_ref()) })
                    .await;
                update_or_log(&this, cx, |store, cx| {
                    if (feed.take)(store, got) {
                        cx.notify();
                    }
                });
            }
            let Some(again) = update_or_log(&this, cx, |store, _| (feed.wanted)(store)) else {
                return;
            };
            if again {
                let timer = cx.background_executor().timer(feed.every);
                smol::future::or(
                    async {
                        timer.await;
                    },
                    kicked.wait(),
                )
                .await;
            } else {
                kicked.wait().await;
            }
        }
    })
    .detach();
}
