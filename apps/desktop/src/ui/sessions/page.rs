//! A session's page, live: its head (status, how long it has been running, where, its tasks),
//! the transcript turn by turn with the model's words appearing as they are typed, the step it
//! is on pinned at the foot, and a box to answer it or tell it to carry on. It follows the
//! newest step until the user scrolls up (Live brings it back); a finished one opens at its
//! top. Stop on the assistant's session asks first and drops what is queued; on the user's own
//! it ends only the turn (⌘.), and its settings change the next turn's model and effort.
//!
//! The same page shows a helper agent's own steps (read only), an external thread (read only, its
//! latest steps every 10 s) and a new session's draft (its first message opens it). It shows
//! the place of the page its window's column has on top.

use std::collections::HashSet;

use emberhut_api::runner::actions::SessionSettings;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, Entity, FocusHandle, ScrollHandle, Subscription, WeakEntity, Window, div,
    point, px,
};

use super::reply::{self, Reply};
use super::{CONTEXT, NewSession, StopTurn, head, live, t3_page, transcript};
use crate::store::sessions::files::SessionFiles;
use crate::store::sessions::places::{SessionKey, SessionPlace};
use crate::store::sessions::sheet::DraftSession;
use crate::store::sessions::summary::{self, Summary};
use crate::store::sessions::t3::T3Watch;
use crate::store::sessions::watch::SessionWatch;
use crate::store::{Page, Store};
use crate::ui::column::Column;
use crate::ui::palette::pal;
use crate::ui::widgets::scroller;

/// How often a running session's page redraws, for its timers.
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// How close to the end the page counts as at its newest step.
const NEAR_END: f32 = 80.;

/// What the page shows now.
enum Shown {
    /// Nothing (the column shows another page).
    Nothing,
    /// A session, or a helper's steps.
    Session(Entity<SessionWatch>),
    /// A new session's draft; once its first message opened it, the session.
    Draft {
        draft: Entity<DraftSession>,
        opened: Option<Entity<SessionWatch>>,
    },
    /// An external thread.
    Thread(Entity<T3Watch>),
    /// A thread of work in flight not on a runner, read only.
    Work(emberhut_api::ids::WorkId),
}

/// A session's page.
pub(crate) struct SessionPage {
    /// The page itself, for the menus' callbacks.
    pub(super) this: WeakEntity<SessionPage>,
    pub(super) store: Entity<Store>,
    shown_key: Option<SessionKey>,
    shown: Shown,
    /// What the page heard of the session it shows (dropped with it).
    watching: Vec<Subscription>,
    pub(super) reply: Reply,
    pub(super) files: Entity<SessionFiles>,
    pub(super) scroll: ScrollHandle,
    focus: FocusHandle,
    /// The page stays on the newest step; only the user's own scrolling up turns it off.
    pub(super) following: bool,
    /// The folded parts opened, by key.
    open: HashSet<String>,
    /// Stop was pressed on a session that asks first.
    pub(super) confirm_stop: bool,
    /// Dictation into the box of the user's own session.
    pub(super) dictation: crate::dictation::Dictation,
    /// The steps and typed words last drawn, to follow what comes.
    drawn: (u64, usize),
    /// Whether the session's first load was in when last drawn.
    loaded: bool,
}

impl SessionPage {
    /// The page over `store`, showing what `column` has on top.
    pub(crate) fn new(
        store: Entity<Store>,
        column: &Entity<Column>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe_in(column, window, |this, column, window, cx| {
            let key = match column.read(cx).current() {
                Some(Page::Session(key)) => Some(key),
                _ => None,
            };
            this.show(key, window, cx);
        })
        .detach();
        cx.observe_in(&store, window, |this, _, window, cx| {
            this.follow_kind(window, cx);
            cx.notify();
        })
        .detach();
        let files = store.update(cx, |store, cx| store.session_files(cx));
        cx.observe(&files, |_, _, cx| cx.notify()).detach();
        let mut page = Self {
            this: cx.entity().downgrade(),
            reply: Reply::new(false, window, cx),
            store,
            shown_key: None,
            shown: Shown::Nothing,
            watching: Vec::new(),
            files,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            following: true,
            open: HashSet::new(),
            confirm_stop: false,
            dictation: crate::dictation::Dictation::system(),
            drawn: (0, 0),
            loaded: false,
        };
        let key = match column.read(cx).current() {
            Some(Page::Session(key)) => Some(key),
            _ => None,
        };
        page.show(key, window, cx);
        page
    }

    /// Shows the place of `key` (none: nothing, and the session it showed is let go).
    fn show(&mut self, key: Option<SessionKey>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if key == self.shown_key {
            return;
        }
        self.shown_key = key;
        self.watching.clear();
        self.open.clear();
        self.confirm_stop = false;
        self.following = true;
        self.loaded = false;
        self.drawn = (0, 0);
        self.scroll.set_offset(point(px(0.), px(0.)));
        let place = key
            .and_then(|key| self.store.read(cx).session_place(key).cloned())
            .map(|place| self.on_runner(place, cx));
        self.shown = match (key, place) {
            (Some(key), Some(SessionPlace::Draft(_))) => match self.store.read(cx).draft(key) {
                Some(draft) => {
                    self.watching
                        .push(cx.observe_in(&draft, window, |this, draft, window, cx| {
                            this.draft_changed(&draft, window, cx);
                        }));
                    let opened = draft.read(cx).opened().cloned();
                    let opened = opened.map(|id| self.watch(&id, window, cx));
                    Shown::Draft { draft, opened }
                }
                None => Shown::Nothing,
            },
            (_, Some(SessionPlace::Session(reference))) => {
                let watch = self
                    .store
                    .update(cx, |store, cx| store.watch_session(&reference, cx));
                self.watching
                    .push(cx.observe_in(&watch, window, |this, watch, window, cx| {
                        this.watch_changed(&watch, window, cx);
                    }));
                Shown::Session(watch)
            }
            (_, Some(SessionPlace::Thread { machine, thread })) => {
                let watch = self
                    .store
                    .update(cx, |store, cx| store.watch_t3(machine, thread, cx));
                self.watching
                    .push(cx.observe(&watch, |_, _, cx| cx.notify()));
                Shown::Thread(watch)
            }
            (_, Some(SessionPlace::Work(id))) => Shown::Work(id),
            _ => Shown::Nothing,
        };
        if let Some(key) = key {
            Self::tick_while_running(key, cx);
        }
        let direct = self.is_direct(cx) || matches!(self.shown, Shown::Draft { .. });
        self.reply = Reply::new(direct, window, cx);
        self.files = self.store.update(cx, |store, cx| store.session_files(cx));
        self.watching
            .push(cx.observe(&self.files, |_, _, cx| cx.notify()));
        cx.notify();
    }

    /// Words the box for the kind of session shown, once its head or the state says it.
    fn follow_kind(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let direct = self.is_direct(cx) || matches!(self.shown, Shown::Draft { .. });
        self.reply.set_direct(direct, window, cx);
    }

    /// A working row on a runner is its session.
    fn on_runner(&self, place: SessionPlace, cx: &gpui_kit::App) -> SessionPlace {
        let SessionPlace::Work(id) = &place else {
            return place;
        };
        let row = self
            .store
            .read(cx)
            .state()
            .and_then(|state| state.working.iter().find(|row| &row.id == id));
        match row {
            Some(row)
                if row
                    .machine
                    .as_ref()
                    .is_some_and(|m| m.as_str().starts_with("runner")) =>
            {
                row.thread.clone().map_or(place, |thread| {
                    SessionPlace::Session(crate::store::sessions::view::SessionRef::whole(
                        emberhut_api::ids::SessionId::from(thread),
                    ))
                })
            }
            _ => place,
        }
    }

    /// While `key` shows, redraws the page every second its session runs, so its timers tick.
    fn tick_while_running(key: SessionKey, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this.update(cx, |this, cx| {
                    if this.shown_key != Some(key) {
                        return false;
                    }
                    if this.summary(cx).is_some_and(|s| s.running) {
                        cx.notify();
                    }
                    true
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Follows the opened session of a draft.
    fn watch(
        &mut self,
        id: &emberhut_api::ids::SessionId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<SessionWatch> {
        let reference = crate::store::sessions::view::SessionRef::whole(id.clone());
        let watch = self
            .store
            .update(cx, |store, cx| store.watch_session(&reference, cx));
        self.watching
            .push(cx.observe_in(&watch, window, |this, watch, window, cx| {
                this.watch_changed(&watch, window, cx);
            }));
        watch
    }

    fn draft_changed(
        &mut self,
        draft: &Entity<DraftSession>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let unsent = draft.update(cx, |draft, _| draft.take_unsent());
        if let Some((text, files)) = unsent {
            self.reply.restore(&text, window, cx);
            self.files.update(cx, |box_, cx| box_.put_back(files, cx));
        }
        let opened = draft.read(cx).opened().cloned();
        if let (Some(id), Shown::Draft { opened: None, .. }) = (opened, &self.shown) {
            let watch = self.watch(&id, window, cx);
            if let Shown::Draft { opened, .. } = &mut self.shown {
                *opened = Some(watch);
            }
            self.reply.clear(window, cx);
        }
        cx.notify();
    }

    fn watch_changed(
        &mut self,
        watch: &Entity<SessionWatch>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let unsent = watch.update(cx, |watch, _| watch.take_unsent());
        if let Some((text, files)) = unsent {
            self.reply.restore(&text, window, cx);
            self.files.update(cx, |box_, cx| box_.put_back(files, cx));
        }
        self.follow_kind(window, cx);
        let sent = watch.read(cx).sent();
        if sent != self.reply.sent_seen() {
            self.reply.sent(sent, window, cx);
            self.following = true;
        }
        let view = watch.read(cx).view();
        let now = (
            view.steps().last().map_or(0, |s| s.seq),
            view.typing().len() / 40,
        );
        if !self.loaded && view.is_loaded() {
            self.loaded = true;
            let running = summary::summary(view, self.store.read(cx).state()).running;
            self.following = running;
            if running {
                self.scroll.scroll_to_bottom();
            }
        } else if now != self.drawn && self.following {
            self.scroll.scroll_to_bottom();
        }
        self.drawn = now;
        cx.notify();
    }

    /// The session shown: a session's, or a draft's once it opened.
    pub(super) fn session(&self) -> Option<&Entity<SessionWatch>> {
        match &self.shown {
            Shown::Session(watch) => Some(watch),
            Shown::Draft { opened, .. } => opened.as_ref(),
            Shown::Nothing | Shown::Thread(_) | Shown::Work(_) => None,
        }
    }

    /// The page head of the session shown.
    pub(super) fn summary(&self, cx: &gpui_kit::App) -> Option<Summary> {
        let watch = self.session()?;
        Some(summary::summary(
            watch.read(cx).view(),
            self.store.read(cx).state(),
        ))
    }

    fn is_direct(&self, cx: &gpui_kit::App) -> bool {
        self.summary(cx).is_some_and(|s| s.direct)
    }

    /// Whether the folded part `key` is open.
    pub(super) fn is_open(&self, key: &str) -> bool {
        self.open.contains(key)
    }

    /// Opens or folds the part `key`.
    pub(super) fn toggle(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        if !self.open.remove(key) {
            self.open.insert(key.to_owned());
        }
        cx.notify();
    }

    /// The user scrolled: following stops when they leave the end, and starts again when they
    /// come back to it.
    fn scrolled(&mut self, cx: &mut Context<'_, Self>) {
        let at_end = -self.scroll.offset().y >= self.scroll.max_offset().y - px(NEAR_END);
        if at_end != self.following {
            self.following = at_end;
            cx.notify();
        }
    }

    /// Starts dictating into the box, opening the microphone off the UI thread, and takes the
    /// voice's loudness while it records.
    pub(super) fn start_dictation(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let Some(opening) = self.dictation.begin() else {
            return;
        };
        let (session, microphone) = (opening.session, opening.microphone);
        let opened = cx.background_spawn(async move { microphone.start() });
        cx.spawn_in(window, async move |this, cx| {
            let opened = opened.await;
            let updated = this.update_in(cx, |this, window, cx| {
                let unwanted = this.dictation.opened(session, opened);
                let_go(unwanted, cx);
                if this.dictation.is_listening() {
                    Self::follow_levels(session, window, cx);
                }
                cx.notify();
            });
            if let Err(error) = updated {
                log::debug!("the page closed while the microphone opened: {error}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Takes a loudness reading every tick while recording `session`, and redraws; stops the
    /// recording at the longest the server takes.
    fn follow_levels(session: u64, window: &Window, cx: &mut Context<'_, Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(crate::dictation::TICK).await;
                let going = this.update_in(cx, |this, window, cx| {
                    if !this.dictation.is_recording(session) {
                        return false;
                    }
                    this.dictation.tick();
                    if this.dictation.is_full() {
                        this.stop_dictation(window, cx);
                    }
                    cx.notify();
                    this.dictation.is_recording(session)
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Stops dictating: the recording is finished and turned into text off the UI thread, and
    /// the words, once the server has them, come after what is typed.
    pub(super) fn stop_dictation(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let Some(stopped) = self.dictation.stop() else {
            cx.notify();
            return;
        };
        let session = stopped.session;
        let said = self
            .store
            .update(cx, |store, cx| store.transcribe(stopped.recording, cx));
        cx.spawn_in(window, async move |this, cx| {
            let said = said.await;
            let updated = this.update_in(cx, |this, window, cx| {
                if let Some(said) = this.dictation.finish(session, said) {
                    let typed = this.reply.text(cx);
                    let text = crate::dictation::append(&typed, &said);
                    this.reply.restore(&text, window, cx);
                }
                cx.notify();
            });
            if let Err(error) = updated {
                log::debug!("the page closed before the dictation's words came: {error}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Drops the recording unused.
    pub(super) fn drop_dictation(&mut self, cx: &mut Context<'_, Self>) {
        let_go(self.dictation.discard(), cx);
        cx.notify();
    }

    /// Back to the newest step, following it again (the Live button).
    pub(super) fn back_to_live(&mut self, cx: &mut Context<'_, Self>) {
        self.following = true;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Sends what the box holds: to the session, or as a draft's first message.
    ///
    /// While a message is on its way (or the draft is being opened) the box keeps what it holds:
    /// nothing is taken from it until the message is.
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = self.reply.text(cx);
        if self.files.read(cx).is_uploading() || self.is_busy(cx) {
            return;
        }
        let files = self.files.read(cx).files().to_vec();
        if text.trim().is_empty() && files.is_empty() {
            return;
        }
        let taken = match &self.shown {
            Shown::Draft {
                draft,
                opened: None,
            } => draft.update(cx, |draft, cx| draft.open(&text, files, cx)),
            _ => match self.session() {
                Some(watch) => watch.update(cx, |watch, cx| watch.send(&text, files, cx)),
                None => false,
            },
        };
        if !taken {
            return;
        }
        // Taken: only now does the box let go of its words and files.
        self.files.update(cx, SessionFiles::clear);
        self.following = true;
        self.reply.sending(window, cx);
    }

    /// Whether a message from the box is on its way: the session's, or the draft's first.
    pub(super) fn is_busy(&self, cx: &gpui_kit::App) -> bool {
        match &self.shown {
            Shown::Draft {
                draft,
                opened: None,
            } => draft.read(cx).is_opening(),
            _ => self
                .session()
                .is_some_and(|watch| watch.read(cx).is_sending()),
        }
    }

    /// An answer picked on the session's question card.
    pub(super) fn answer(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        if let Some(watch) = self.session() {
            watch.update(cx, |watch, cx| watch.send(text, Vec::new(), cx));
            self.following = true;
        }
    }

    /// Stop: on the user's own session only the running turn; on the assistant's it asks
    /// first (the confirm card).
    pub(super) fn stop_pressed(&mut self, cx: &mut Context<'_, Self>) {
        let Some(head) = self.summary(cx) else {
            return;
        };
        if head.direct {
            self.stop_turn(cx);
        } else if head.running {
            self.confirm_stop = true;
            cx.notify();
        }
    }

    /// Stops the running turn; what is queued runs next.
    pub(super) fn stop_turn(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(watch) = self.session() {
            watch.update(cx, SessionWatch::interrupt);
        }
    }

    /// Stops the session: the step it's on ends and nothing queued runs (asked first).
    pub(super) fn stop_all(&mut self, cx: &mut Context<'_, Self>) {
        self.confirm_stop = false;
        if let Some(watch) = self.session() {
            watch.update(cx, SessionWatch::stop);
        }
        cx.notify();
    }

    /// Changes the next turn's model or effort.
    pub(super) fn change(&mut self, settings: SessionSettings, cx: &mut Context<'_, Self>) {
        if let Some(watch) = self.session() {
            watch.update(cx, |watch, cx| watch.change(settings, cx));
        }
    }

    fn stop_shortcut(&mut self, _: &StopTurn, _: &mut Window, cx: &mut Context<'_, Self>) {
        if self.summary(cx).is_some_and(|s| s.direct && s.running) {
            self.stop_turn(cx);
        } else {
            cx.propagate();
        }
    }

    fn session_view(&self, watch: &Entity<SessionWatch>, cx: &mut Context<'_, Self>) -> Div {
        let p = pal(cx);
        let head = summary::summary(watch.read(cx).view(), self.store.read(cx).state());
        let body = transcript::body(self, watch, &head, &p, cx);
        let foot = live::foot(self, watch, &head, &p, cx);
        let live_button = (head.running && watch.read(cx).view().is_loaded() && !self.following)
            .then(|| live::back_to_live(&p, cx));
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(head::bar(self, watch, &head, &p, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        scroller("session-scroll", &self.scroll)
                            .on_scroll(cx.listener(|this, _, _, cx| this.scrolled(cx)))
                            .child(body),
                    )
                    .children(live_button),
            )
            .child(foot)
            .when(self.confirm_stop, |d| d.child(head::confirm(&p, cx)))
    }

    fn draft_view(&self, draft: &Entity<DraftSession>, cx: &mut Context<'_, Self>) -> Div {
        let p = pal(cx);
        let read = draft.read(cx);
        let state = self.store.read(cx).state();
        let card = head::draft_card(
            &p,
            &read.setup().where_line(),
            &read.setup().what_line(state),
            read.is_opening(),
            read.problem(),
            cx,
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().p(px(14.)).child(card))
            .child(reply::composer(
                self,
                true,
                false,
                "What should it do?",
                &p,
                cx,
            ))
    }
}

impl Render for SessionPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let content: AnyElement = match &self.shown {
            Shown::Nothing => div().into_any_element(),
            Shown::Session(watch)
            | Shown::Draft {
                opened: Some(watch),
                ..
            } => {
                let watch = watch.clone();
                self.session_view(&watch, cx).into_any_element()
            }
            Shown::Draft {
                draft,
                opened: None,
            } => {
                let draft = draft.clone();
                self.draft_view(&draft, cx).into_any_element()
            }
            Shown::Thread(watch) => {
                let watch = watch.clone();
                t3_page::page(self, &watch, cx).into_any_element()
            }
            Shown::Work(id) => t3_page::work(self, id, cx).into_any_element(),
        };
        div()
            .id("session-page")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::stop_shortcut))
            .on_action(cx.listener(|_, _: &NewSession, _, cx| cx.propagate()))
            .size_full()
            .child(content)
    }
}

/// Lets go of a recording no longer wanted on a background thread: stopping the device waits
/// for its audio thread.
fn let_go(
    recording: Option<Box<dyn crate::dictation::mic::Recording>>,
    cx: &mut Context<'_, SessionPage>,
) {
    if let Some(recording) = recording {
        cx.background_spawn(async move { drop(recording) }).detach();
    }
}
