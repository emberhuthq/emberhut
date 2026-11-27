//! Sessions, every room's: the agents working now, the sessions running, the external threads still
//! running and the recent ones (waiting on the user first), under a filter bar (room, who runs
//! it, where) kept on this device, with New session in the head (⇧⌘N). A row opens its page in
//! the column: a session live ([`page`]), a helper agent's steps, an external thread, or a new
//! session's draft. The same screen is the Sessions window of its own (⇧⌘T, [`window`]), where
//! sessions and tasks open in place.

mod filter_bar;
mod head;
mod live;
mod marks;
pub(crate) mod page;
mod reply;
mod rows;
mod sheet;
mod spark;
mod t3_page;
#[cfg(test)]
mod tests;
mod todos;
mod transcript;
pub(crate) mod window;

use std::collections::HashSet;

use emberhut_api::ids::{SessionId, TaskId};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, FocusHandle, FontWeight, KeyBinding, ScrollHandle, Window, actions, div, px,
};

pub(crate) use self::marks::state_mark;
pub(crate) use self::page::SessionPage;
use self::rows::{Rows, counted};
use self::sheet::NewSessionView;
use super::a11y::Accessible as _;
use super::column;
use super::controls::tab_stop;
use super::palette::{Pal, pal};
use super::tasks::marks::glyph;
use super::widgets::scroller;
use crate::store::sessions::filter::SessionFilter;
use crate::store::sessions::lists::{self, Screen};
use crate::store::sessions::places::SessionPlace;
use crate::store::{Page, Store};

actions!(
    sessions,
    [
        /// Opens the New session sheet.
        NewSession,
        /// Stops the running turn of the user's own session; what is queued runs next.
        StopTurn,
        /// Opens the Sessions window.
        OpenSessionsWindow,
    ]
);

/// The key context of the Sessions screen and a session's page.
pub(crate) const CONTEXT: &str = "Sessions";

/// Binds the sessions' keys: ⇧⌘N New session and ⌘. Stop this turn where they are, ⇧⌘T the
/// Sessions window anywhere (Ctrl elsewhere).
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-n", NewSession, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-n", NewSession, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-.", StopTurn, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-.", StopTurn, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-t", OpenSessionsWindow, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-t", OpenSessionsWindow, None),
    ]);
    sheet::bind_keys(cx);
}

/// Where a click on a sessions row or link goes.
#[derive(Clone, Debug)]
pub(crate) enum Open {
    /// A session's page (a session, a helper's steps, an external thread, a draft).
    Place(SessionPlace),
    /// A task's page.
    Task(TaskId),
}

fn page_of(store: &Entity<Store>, open: Open, cx: &mut App) -> Page {
    match open {
        Open::Place(place) => Page::Session(store.update(cx, |store, _| store.session_key(place))),
        Open::Task(task) => Page::Task(task),
    }
}

/// Opens `open` over what the window's column shows; Back returns there.
pub(crate) fn push(store: &Entity<Store>, open: Open, window: &Window, cx: &mut App) {
    let page = page_of(store, open, cx);
    column::push(page, window, cx);
}

/// Opens `open` afresh in the window's column (the sidebar's rows).
pub(crate) fn open(store: &Entity<Store>, open: Open, window: &Window, cx: &mut App) {
    let page = page_of(store, open, cx);
    column::open(page, window, cx);
}

/// `row`, made a link to `place` (a task page's agent and session rows).
pub(crate) fn link(
    store: &Entity<Store>,
    place: SessionPlace,
    id: impl Into<gpui_kit::ElementId>,
    row: Div,
    p: &Pal,
) -> gpui_kit::Stateful<Div> {
    let store = store.clone();
    tab_stop(
        div()
            .id(id)
            // Read as its row's words, as a button that opens the session.
            .role(gpui_kit::Role::Button)
            .hint("Opens its session")
            .border_1()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(row)
            .on_click(move |_, window, cx| push(&store, Open::Place(place.clone()), window, cx)),
        p,
    )
}

/// A thread's link on a row of work in flight (a Needs you card with no task): its session on
/// a runner, else what it was asked and said.
pub(crate) fn work_link(
    store: &Entity<Store>,
    id: emberhut_api::ids::WorkId,
    p: &Pal,
) -> impl IntoElement {
    let store = store.clone();
    tab_stop(
        div()
            .id(gpui_kit::ElementId::Name(format!("work-open-{id}").into()))
            .button_named("Open the thread")
            .border_1()
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.))
            .text_color(p.accent)
            .cursor_pointer()
            .child("Open the thread")
            .child(glyph(IconName::ChevronRight, p.accent, 11.))
            .on_click(move |_, window, cx| {
                push(
                    &store,
                    Open::Place(SessionPlace::Work(id.clone())),
                    window,
                    cx,
                );
            }),
        p,
    )
}

/// The Sessions screen.
pub(crate) struct SessionsView {
    store: Entity<Store>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    /// The finished rows opened to their last answer.
    open_rows: HashSet<SessionId>,
    /// The New session sheet, while it is up.
    sheet: Option<Entity<NewSessionView>>,
}

impl SessionsView {
    /// The screen over `store`.
    pub(crate) fn new(store: Entity<Store>, cx: &mut Context<'_, Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        // The running rows' timers tick each second something runs.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let held = this.update(cx, |this, cx| {
                    let runs = this
                        .store
                        .read(cx)
                        .state()
                        .is_some_and(|state| lists::running_now(state) > 0);
                    if runs {
                        cx.notify();
                    }
                });
                if held.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            store,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            open_rows: HashSet::new(),
            sheet: None,
        }
    }

    /// Opens the New session sheet.
    pub(crate) fn new_session(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.sheet.is_some() {
            return;
        }
        let store = self.store.clone();
        let this = cx.entity().downgrade();
        let view = cx.new(|cx| {
            NewSessionView::new(
                store,
                Box::new(move |_window, cx| {
                    if let Some(this) = this.upgrade() {
                        this.update(cx, |this, cx| {
                            this.sheet = None;
                            cx.notify();
                        });
                    }
                }),
                window,
                cx,
            )
        });
        view.update(cx, |view, cx| view.focus(window, cx));
        self.sheet = Some(view);
        cx.notify();
    }

    /// Gives the screen the keyboard, for its shortcuts.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn head(p: &Pal, screen: Option<&Screen<'_>>, cx: &Context<'_, Self>) -> Div {
        let running = screen.map_or(0, |s| s.working.len() + s.running.len() + s.threads.len());
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(52.))
            .px(px(24.))
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .child("Sessions"),
            )
            .when(running > 0, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.success)
                        .child(format!("{running} running")),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("new-session")
                    .ghost()
                    .small()
                    .icon(glyph(IconName::Plus, p.muted, 15.))
                    .accessibility_label("New session")
                    .tooltip("New session (⇧⌘N)")
                    .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
            )
    }

    fn body(&self, screen: &Screen<'_>, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let read = self.store.read(cx);
        let now = rows::now();
        let state = read.state();
        let key = |task: TaskId| read.task_key(task);
        let rows = Rows {
            store: &self.store,
            p,
            now: &now,
            key: &key,
            accounts: state.map_or(&[], |state| state.accounts.as_slice()),
        };
        let filter_empty = read.session_filter().is_empty();
        let mut body = div()
            .flex()
            .flex_col()
            .px(px(12.))
            .py(px(4.))
            .child(filter_bar::bar(
                &self.store,
                &screen.rooms,
                &screen.picks,
                p,
                cx,
            ));
        if !screen.working.is_empty() {
            body = body.child(counted(p, "Working now", screen.working.len(), true));
            for (has_lead, group) in lists::by_task(&screen.working) {
                for agent in group {
                    body = body.child(rows.agent(agent, !has_lead, cx));
                }
            }
        }
        if screen.is_empty() {
            body = body.child(Self::empty(p, filter_empty, cx));
        }
        if !screen.running.is_empty() {
            body = body.child(counted(p, "Running", screen.running.len(), true));
            for session in &screen.running {
                body = body.child(rows.running(session, cx));
            }
        }
        if !screen.threads.is_empty() {
            body = body.child(crate::ui::widgets::section_title(
                p,
                "Still in the coding-thread app",
            ));
            for thread in &screen.threads {
                body = body.child(rows.thread(thread, cx));
            }
            body = body.child(
                div()
                    .px(px(10.))
                    .pb(px(4.))
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(format!(
                        "Read only: these finish in the coding-thread app. New work runs on {}'s runner.",
                        crate::brand::APP_NAME
                    )),
            );
        }
        if !screen.recent.is_empty() {
            body = body.child(crate::ui::widgets::section_title(p, "Recent"));
            body = self.recent(body, &rows, screen, cx);
        }
        if !filter_empty && screen.hidden > 0 && state.is_some() {
            let store = self.store.clone();
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(10.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(format!("{} more hidden by the filter", screen.hidden))
                    .child(tab_stop(
                        div()
                            .id("sessions-show-all")
                            .button_named("Show all")
                            .border_1()
                            .rounded(px(6.))
                            .px(px(6.))
                            .text_color(p.accent)
                            .cursor_pointer()
                            .child("Show all")
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| {
                                    store.set_session_filter(SessionFilter::default(), cx);
                                });
                            }),
                        p,
                    )),
            );
        }
        body
    }

    /// The recent rows, each with its chevron that opens its last answer.
    fn recent(
        &self,
        mut body: Div,
        rows: &Rows<'_>,
        screen: &Screen<'_>,
        cx: &Context<'_, Self>,
    ) -> Div {
        for session in &screen.recent {
            let open = self.open_rows.contains(&session.id);
            let (this, id) = (cx.entity().downgrade(), session.id.clone());
            let toggle: rows::Toggle = Box::new(move |_, cx| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        if !this.open_rows.remove(&id) {
                            this.open_rows.insert(id.clone());
                        }
                        cx.notify();
                    });
                }
            });
            body = body.child(rows.line(session, open, Some(toggle), cx));
        }
        body
    }

    fn empty(p: &Pal, unfiltered: bool, cx: &Context<'_, Self>) -> Div {
        let (title, words) = if unfiltered {
            (
                "No sessions yet",
                format!(
                    "Open Claude Code on the Linux PC or the Mac and talk to it yourself. {}'s project work shows here too.",
                    crate::brand::APP_NAME
                ),
            )
        } else {
            (
                "Nothing matches the filter",
                "Take the filter off to see everything.".to_owned(),
            )
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .py(px(48.))
            .px(px(24.))
            .child(glyph(IconName::SquareTerminal, p.faint, 28.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .text_center()
                    .child(words),
            )
            .when(unfiltered, |d| {
                d.child(
                    Button::new("sessions-empty-new")
                        .primary()
                        .small()
                        .label("New session")
                        .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
                )
            })
    }
}

impl Render for SessionsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let read = self.store.read(cx);
        let screen = read.sessions_screen();
        let content = match &screen {
            Some(screen) => self.body(screen, &p, cx).into_any_element(),
            None => div()
                .p(px(24.))
                .text_color(p.muted)
                .child("Loading…")
                .into_any_element(),
        };
        let head = Self::head(&p, screen.as_ref(), cx);
        div()
            .id("sessions")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NewSession, window, cx| this.new_session(window, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(head)
            .child(scroller("sessions-scroll", &self.scroll).child(content))
            .children(self.sheet.clone())
    }
}
