//! Moving the app to another server while it runs, with no restart: the first-run setup's hut
//! may answer at another address than the one the app was launched with (its port was taken).
//!
//! [`Store::switch_server`] keeps the address as `config.json`'s `server`, so the next launch
//! talks to it too, then has the [`Connector`] make a backend for it the way the launch made
//! the first one, and reconnects: the state poll stops, the backend and what the old server
//! said go, the live stream follows the new server, and the poll starts again. `EMBERHUT_URL`
//! wins over a move, as it wins over the file at launch: the store then stays where it is.
//!
//! The loops that fetch now and then (the machines, the switches, the approvals…) take the
//! store's backend at each round, so they follow by themselves. A page that holds a backend of
//! its own (a session's, a party's) keeps the old one until it closes: a move is meant for the
//! setup, before any of them is open.

use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Task};

use super::on_disk::DiskCache;
use super::sign_in::same_server;
use super::{Store, gate, poll};
use crate::backend::{Backend, Error};
use crate::config::ServerUrl;

/// Makes the backend for another server, as the launch made the first.
pub(crate) trait Connector: Send + Sync {
    /// The backend for `server` (its session kept as the app keeps sessions), and the cache of
    /// the chats kept for it. It may block: it runs off the UI thread.
    ///
    /// # Errors
    ///
    /// A client couldn't be made.
    fn connect(&self, server: &ServerUrl) -> Result<Connected, Error>;
}

/// A backend for another server, made by a [`Connector`].
pub(crate) struct Connected {
    /// The backend.
    pub(crate) backend: Arc<dyn Backend>,
    /// The chats kept for its server (none without a cache directory).
    pub(crate) disk: Option<DiskCache>,
}

/// How the store moves to another server.
#[derive(Clone)]
pub(crate) struct Reconnect {
    /// Makes the new backend; `None` when this launch can't move (the self-check, most tests).
    connector: Option<Arc<dyn Connector>>,
    /// The environment variable that named the server at launch, when one did: it wins.
    pinned_by: Option<&'static str>,
}

impl Reconnect {
    /// No move: the store stays on the server it was made with.
    #[must_use]
    pub(crate) fn none() -> Self {
        Self {
            connector: None,
            pinned_by: None,
        }
    }

    /// Moves made by `connector`, unless `pinned_by` (an environment variable that named the
    /// server at launch) is set.
    #[must_use]
    pub(crate) fn new(connector: Arc<dyn Connector>, pinned_by: Option<&'static str>) -> Self {
        Self {
            connector: Some(connector),
            pinned_by,
        }
    }
}

impl Reconnect {
    /// Whether the variable that named the server at launch pins it.
    #[must_use]
    pub(super) fn is_pinned(&self) -> bool {
        self.pinned_by.is_some()
    }
}

impl Store {
    /// The backend the store talks through now.
    #[must_use]
    pub(crate) fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// Moves the app to `server` while it runs: kept in `config.json`, then the store
    /// reconnects to it. Nothing is done when it is the server the app talks to already (the
    /// same address, or both on this computer at one port: `localhost` and `127.0.0.1` are one
    /// place). The task ends once the store talks to `server`, or with why it doesn't, in
    /// words: the variable that named the server at launch wins, the file couldn't be written,
    /// or no backend could be made.
    pub(crate) fn switch_server(
        &mut self,
        server: ServerUrl,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<(), String>> {
        if same_server(&server, self.server()) {
            return Task::ready(Ok(()));
        }
        let current = self.server().clone();
        if let Some(variable) = self.reconnect.pinned_by {
            return Task::ready(Err(format!(
                "Emberhut started here at {server}, but {variable} tells this app to look for \
                 your ember at {current}, so it stays there. Unset {variable} to use the one \
                 here."
            )));
        }
        let Some(connector) = self.reconnect.connector.clone() else {
            return Task::ready(Err(format!(
                "Emberhut started here at {server}, and this app looks for your ember at \
                 {current}. Open the app again to use it."
            )));
        };
        let files = self.files.clone();
        cx.spawn(async move |this, cx| {
            let made = cx.background_spawn({
                let server = server.clone();
                async move {
                    if let Some(files) = &files {
                        files.write_server(&server).map_err(|error| {
                            format!(
                                "Emberhut started here at {server}, but this app couldn't keep \
                                 its address: {error}"
                            )
                        })?;
                    }
                    connector.connect(&server).map_err(|error| {
                        format!("Emberhut started here at {server}, but this app can't reach it: {error}")
                    })
                }
            });
            let connected = made.await?;
            this.update(cx, |store, cx| store.reconnect_to(connected, cx))
                .map_err(|_| "The app closed while it moved to Emberhut here.".to_owned())
        })
    }

    /// Talks to `connected` from now on: the poll stops (a reply on its way is dropped with
    /// it), the backend and the chats' cache are swapped, what the old server said goes, the
    /// live stream follows the open room on the new one, and the poll starts again if it ran.
    fn reconnect_to(&mut self, connected: Connected, cx: &mut Context<'_, Self>) {
        let Connected { backend, disk } = connected;
        log::info!("talking to {} now", backend.server());
        let polling = self.poll.take().is_some();
        let (kicker, waiting) = poll::kicker();
        self.kicker = kicker;
        self.kicked = Some(waiting);
        self.backend = backend;
        self.disk = disk.map(Arc::new);
        self.live_feed.switch(Arc::clone(&self.backend), self.room);
        self.state = None;
        self.etag = None;
        self.live = None;
        self.problem = None;
        self.needs_sign_in = false;
        self.report = None;
        self.chats = super::Chats::default();
        self.door = gate::Door::default();
        self.failing = gate::Failing::default();
        if polling {
            self.start_poll(cx);
        }
        self.machines.kick();
        cx.notify();
    }
}
