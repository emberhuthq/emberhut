//! The fallback server, as the store shows it: whether the app turned to it because the server
//! stopped answering, and the user's Try again, which turns back only when the server answers.

use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};
use crate::backend::Serving;

/// How the user's last Try again went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Retry {
    /// The server is being asked now.
    pub(crate) trying: bool,
    /// The last ask found it still not answering.
    pub(crate) still_silent: bool,
}

impl Store {
    /// Which server the app talks to: the server, or the fallback server.
    #[must_use]
    pub(crate) fn serving(&self) -> Serving {
        self.backend.serving()
    }

    /// How the user's last Try again went.
    #[must_use]
    pub(crate) fn primary_retry(&self) -> Retry {
        self.primary_retry
    }

    /// Asks whether the server answers again, and turns back to it only if it does; then the
    /// state is polled from whichever server the app is on.
    pub(crate) fn try_primary(&mut self, cx: &mut Context<'_, Self>) {
        if self.primary_retry.trying {
            return;
        }
        self.primary_retry = Retry {
            trying: true,
            still_silent: false,
        };
        cx.notify();
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let back = cx
                .background_spawn(async move { backend.try_primary() })
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                store.primary_retry = Retry {
                    trying: false,
                    still_silent: !back,
                };
                cx.notify();
            });
        })
        .detach();
    }
}
