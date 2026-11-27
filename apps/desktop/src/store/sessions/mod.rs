//! The sessions on the runners, every room's: the lists ([`lists`]) and the Sessions screen's
//! filter ([`filter`], kept on this device), one session followed live ([`watch`], shared by
//! the pages that show it), an external thread's steps ([`t3`]), and a new session of the user's own
//! ([`sheet`]). The logic with no GPUI in it ([`words`], [`lists`], [`filter`], [`view`],
//! [`turns`], [`summary`]) is tested on its own.

pub(crate) mod files;
pub(crate) mod filter;
mod kept;
pub(crate) mod lists;
pub(crate) mod places;
pub(crate) mod sheet;
pub(crate) mod summary;
pub(crate) mod t3;
#[cfg(test)]
mod tests;
pub(crate) mod turns;
pub(crate) mod view;
pub(crate) mod watch;
pub(crate) mod words;

use std::collections::HashMap;
use std::sync::Arc;

use emberhut_api::ids::MachineId;
use gpui_kit::{AppContext as _, Context, Entity, WeakEntity};

use self::files::SessionFiles;
use self::filter::SessionFilter;
use self::kept::Kept;
use self::lists::Screen;
use self::places::{Places, SessionKey, SessionPlace};
use self::sheet::{DraftSession, SessionSheet, Setup};
use self::t3::T3Watch;
use self::view::SessionRef;
use self::watch::SessionWatch;
use super::Store;
use crate::config::AppFiles;
use crate::json_file::Writes;

/// The store's side of the sessions.
pub(crate) struct SessionWork {
    /// The filter and the sheet's choices, kept on this device.
    kept: Kept,
    /// The writes of what is kept to its file, in order.
    kept_writes: Writes,
    /// The sessions followed now, each shared by the pages that show it.
    watches: HashMap<SessionRef, WeakEntity<SessionWatch>>,
    /// What the session pages show, by the key their page carries.
    places: Places,
    /// The new sessions about to open, by their page's key (kept while the app runs, so a
    /// draft that opened stays its session).
    drafts: HashMap<SessionKey, Entity<DraftSession>>,
}

impl SessionWork {
    /// The sessions' side, with what this device kept.
    #[must_use]
    pub(crate) fn new(files: Option<&AppFiles>) -> Self {
        Self {
            kept: kept::load(files),
            kept_writes: Writes::default(),
            watches: HashMap::new(),
            places: Places::default(),
            drafts: HashMap::new(),
        }
    }
}

impl Store {
    /// The Sessions screen's filter.
    #[must_use]
    pub(crate) fn session_filter(&self) -> &SessionFilter {
        &self.sessions.kept.filter
    }

    /// Sets the Sessions screen's filter, and keeps it.
    pub(crate) fn set_session_filter(&mut self, filter: SessionFilter, cx: &mut Context<'_, Self>) {
        if self.sessions.kept.filter == filter {
            return;
        }
        self.sessions.kept.filter = filter;
        self.keep_sessions(cx);
        cx.notify();
    }

    /// Writes what the Sessions screen keeps to its file off the UI thread, numbered, so a
    /// newer write always wins.
    fn keep_sessions(&mut self, cx: &mut Context<'_, Self>) {
        let Some(files) = self.files.clone() else {
            return;
        };
        let (number, written) = self.sessions.kept_writes.next();
        let kept = self.sessions.kept.clone();
        cx.background_spawn(async move { kept::save(&files, &kept, number, &written) })
            .detach();
    }

    /// The Sessions screen as the filter leaves it, once a state came.
    #[must_use]
    pub(crate) fn sessions_screen(&self) -> Option<Screen<'_>> {
        Some(lists::screen(
            self.state.as_ref()?,
            &self.sessions.kept.filter,
        ))
    }

    /// `reference` followed live: the one a page already follows, else a new one. The stream
    /// runs while a page holds what this gives.
    pub(crate) fn watch_session(
        &mut self,
        reference: &SessionRef,
        cx: &mut Context<'_, Self>,
    ) -> Entity<SessionWatch> {
        self.sessions
            .watches
            .retain(|_, watch| watch.upgrade().is_some());
        if let Some(watch) = self
            .sessions
            .watches
            .get(reference)
            .and_then(WeakEntity::upgrade)
        {
            return watch;
        }
        let backend = Arc::clone(&self.backend);
        let watch = cx.new(|cx| SessionWatch::new(reference.clone(), backend, cx));
        self.sessions
            .watches
            .insert(reference.clone(), watch.downgrade());
        watch
    }

    /// A new-session sheet, starting on the choices kept from the last one.
    pub(crate) fn new_session_sheet(&self, cx: &mut Context<'_, Self>) -> Entity<SessionSheet> {
        let (backend, choices) = (Arc::clone(&self.backend), self.sessions.kept.sheet.clone());
        cx.new(|cx| SessionSheet::new(backend, &choices, cx))
    }

    /// The key of the page that shows `place`.
    pub(crate) fn session_key(&mut self, place: SessionPlace) -> SessionKey {
        self.sessions.places.key(place)
    }

    /// What the page of `key` shows.
    #[must_use]
    pub(crate) fn session_place(&self, key: SessionKey) -> Option<&SessionPlace> {
        self.sessions.places.get(key)
    }

    /// A new session's draft page for what the sheet set up; its machine, model and effort are
    /// kept for the next sheet.
    pub(crate) fn draft_session(&mut self, setup: Setup, cx: &mut Context<'_, Self>) -> SessionKey {
        let choices = setup.choices();
        if self.sessions.kept.sheet != choices {
            self.sessions.kept.sheet = choices;
            self.keep_sessions(cx);
        }
        let key = self.sessions.places.key(SessionPlace::Draft(setup.clone()));
        let backend = Arc::clone(&self.backend);
        let draft = cx.new(|_| DraftSession::new(backend, setup));
        self.sessions.drafts.insert(key, draft);
        key
    }

    /// The draft of the page `key`, if it is one.
    #[must_use]
    pub(crate) fn draft(&self, key: SessionKey) -> Option<Entity<DraftSession>> {
        self.sessions.drafts.get(&key).cloned()
    }

    /// An empty box of files for a message to a session.
    pub(crate) fn session_files(&self, cx: &mut Context<'_, Self>) -> Entity<SessionFiles> {
        let backend = Arc::clone(&self.backend);
        cx.new(|_| SessionFiles::new(backend))
    }

    /// Takes `state` as the last one (the views' tests).
    #[cfg(test)]
    pub(crate) fn show_state_for_tests(
        &mut self,
        state: emberhut_api::state::State,
        cx: &mut Context<'_, Self>,
    ) {
        self.state = Some(state);
        cx.notify();
    }

    /// An external thread's steps, asked for again every few seconds while its page holds this.
    pub(crate) fn watch_t3(
        &self,
        machine: MachineId,
        thread: String,
        cx: &mut Context<'_, Self>,
    ) -> Entity<T3Watch> {
        let backend = Arc::clone(&self.backend);
        cx.new(|cx| T3Watch::new(backend, machine, thread, cx))
    }
}
