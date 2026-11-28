//! "Add a hut": another computer joins the ember as a hut. The server makes a one-time code
//! (`POST /api/runners/codes`, ten minutes) with the line to run on that computer
//! (`emberhut-runner install --server … --pair …`); the store then looks at the server's
//! runners every few seconds until a new one shows up (the hut joined) or the code runs out.
//! Opened from the setup's "Add another hut" and the Huts page's "Add a hut".
//!
//! The server says whether its address reaches past this computer (`reachable`). Until the
//! runner dials out to the server, a hut on another computer needs both on one network or
//! tailnet: the panel says so when it doesn't.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::runner::pairing::RunnerCode;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};

/// How often the runners are looked at for the new hut.
const LOOK_EVERY: Duration = Duration::from_secs(3);

/// How long the new hut is waited for: the code's own life.
const WAIT_AT_MOST: Duration = Duration::from_mins(10);

/// Where adding a hut is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AddHut {
    /// Not open.
    #[default]
    Closed,
    /// The code is being made.
    Making,
    /// The code and what to run with it, waiting for the hut.
    Waiting(Box<RunnerCode>),
    /// A new hut joined, by its name.
    Joined(String),
    /// The code ran out before a hut joined.
    Expired,
    /// The server couldn't make a code: why.
    Failed(String),
}

/// The panel's state, and which look is the current one (a newer one wins).
#[derive(Clone, Debug, Default)]
pub(crate) struct AddingHut {
    state: AddHut,
    look: u64,
}

impl Store {
    /// Where adding a hut is.
    #[must_use]
    pub(crate) fn add_hut(&self) -> &AddHut {
        &self.adding_hut.state
    }

    /// Opens "Add a hut": a code is made, and the new hut is waited for.
    pub(crate) fn open_add_hut(&mut self, cx: &mut Context<'_, Self>) {
        self.adding_hut.look += 1;
        let look = self.adding_hut.look;
        self.adding_hut.state = AddHut::Making;
        cx.notify();
        let backend = Arc::clone(&self.backend);
        let made = cx.background_spawn(async move {
            let known: BTreeSet<String> = backend.runners().map_or_else(
                |_| BTreeSet::new(),
                |page| page.runners.into_iter().map(|runner| runner.name).collect(),
            );
            backend.runner_pair_code().map(|code| (code, known))
        });
        cx.spawn(async move |this, cx| {
            let made = made.await;
            let known = match made {
                Ok((code, known)) => {
                    let current = this
                        .update(cx, |store, cx| {
                            if store.adding_hut.look != look {
                                return false;
                            }
                            store.adding_hut.state = AddHut::Waiting(Box::new(code));
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !current {
                        return;
                    }
                    known
                }
                Err(error) => {
                    update_or_log(&this, cx, |store, cx| {
                        if store.adding_hut.look == look {
                            store.adding_hut.state = AddHut::Failed(error.to_string());
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            let tries = WAIT_AT_MOST.as_secs() / LOOK_EVERY.as_secs();
            for _ in 0..tries {
                cx.background_executor().timer(LOOK_EVERY).await;
                let backend = this
                    .read_with(cx, |store, _| Arc::clone(&store.backend))
                    .ok();
                let Some(backend) = backend else { return };
                let page = cx.background_spawn(async move { backend.runners() }).await;
                let joined = page.ok().and_then(|page| {
                    page.runners
                        .into_iter()
                        .map(|runner| runner.name)
                        .find(|name| !known.contains(name))
                });
                let still = this
                    .update(cx, |store, cx| {
                        if store.adding_hut.look != look {
                            return false;
                        }
                        if let Some(name) = joined.clone() {
                            store.adding_hut.state = AddHut::Joined(name);
                            cx.notify();
                            return false;
                        }
                        true
                    })
                    .unwrap_or(false);
                if !still {
                    return;
                }
            }
            update_or_log(&this, cx, |store, cx| {
                if store.adding_hut.look == look {
                    store.adding_hut.state = AddHut::Expired;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Closes "Add a hut" (Done, Cancel); a wait under way stops.
    pub(crate) fn close_add_hut(&mut self, cx: &mut Context<'_, Self>) {
        self.adding_hut.look += 1;
        self.adding_hut.state = AddHut::Closed;
        cx.notify();
    }
}
