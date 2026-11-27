//! Emberhut here at a later launch: when the app talks to the hut and it is out (it ran as the
//! app's child, it crashed, or it was stopped), it is started again while the window shows
//! the usual Connecting; once it answers the poll goes at once. A start that fails shows the
//! hut's words with Try again in Connecting's place, never an endless spinner. Nothing starts
//! when `EMBERHUT_URL` named the server, or for an address that isn't the hut's.

use gpui_kit::Context;

use super::super::sign_in::same_server;
use super::super::{Store, update_or_log};
use super::Relit;

/// How Emberhut here coming back at launch goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Relighting {
    /// Nothing asked, or done.
    #[default]
    Idle,
    /// It is being looked at, or started.
    Starting,
    /// It couldn't be started: why, in the hut's words.
    Failed(String),
}

impl Store {
    /// At launch, and on Try again: starts Emberhut here when the app talks to it and it is out.
    pub(crate) fn relight_home(&mut self, cx: &mut Context<'_, Self>) {
        if self.reconnect.is_pinned() || self.setup.relighting == Relighting::Starting {
            return;
        }
        let relit = self.setup.home.relight(self.server(), cx);
        self.setup.relighting = Relighting::Starting;
        cx.spawn(async move |this, cx| {
            let relit = relit.await;
            update_or_log(&this, cx, |store, cx| store.home_relit(relit, cx));
        })
        .detach();
        cx.notify();
    }

    /// Emberhut here came back as `relit`: the poll goes at once once it answers (at another
    /// address, the app moves there first).
    fn home_relit(&mut self, relit: Relit, cx: &mut Context<'_, Self>) {
        self.setup.relighting = Relighting::Idle;
        match relit {
            Relit::NotHere | Relit::Lit => {}
            Relit::Started(url) if same_server(&url, self.server()) => self.kicker.kick(),
            Relit::Started(url) => {
                let moved = self.switch_server(url, cx);
                cx.spawn(async move |this, cx| {
                    if let Err(why) = moved.await {
                        update_or_log(&this, cx, |store, cx| {
                            store.setup.relighting = Relighting::Failed(why);
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Relit::Failed(why) => {
                log::warn!("the hut didn't start again: {why}");
                self.setup.relighting = Relighting::Failed(why);
            }
        }
        cx.notify();
    }

    /// Why Emberhut here didn't start again at launch, while it didn't.
    #[must_use]
    pub(crate) fn relight_problem(&self) -> Option<&str> {
        match &self.setup.relighting {
            Relighting::Failed(why) => Some(why),
            Relighting::Idle | Relighting::Starting => None,
        }
    }
}
