//! Tasks: one flat list (what needs you, today's, the rest) under a search over every task and
//! the filter bar, with New task in the head, the archived tasks folded at the foot, and an
//! empty state that says whether nothing is open or the filter hides it.
//!
//! On a server that keeps plans, Tasks is the main place: under Needs you come what runs and
//! what is blocked, each row with its plan, and at the foot the sessions with no task as items
//! of their own (waiting, running, then the last day's), with a way to every session, so no
//! work leaves the screen.

mod filter_bar;
mod loose;
pub(crate) mod marks;
pub(crate) mod plan_marks;
pub(crate) mod row;
pub(crate) mod swipe;
pub(crate) mod tick;

use emberhut_api::health_today::HealthToday;
use emberhut_api::state::State;
use emberhut_api::tasks::Issue;
use gpui_kit::Focusable as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FontWeight, ScrollHandle, SharedString, Window, div, px};
use jiff::civil::Date;

use self::marks::{glyph, progress, thousands};
use self::row::Look;
use self::swipe::RowSwipes;
use super::controls::tab_stop;
use super::palette::{Pal, pal};
use super::widgets::scroller;
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::lookup;
use crate::store::tasks::clock;
use crate::store::tasks::filter::TaskFilter;
use crate::store::tasks::list::{self, TaskList};
use crate::store::tasks::loose::{self as loose_sessions, Loose};
use crate::store::tasks::search::Found;

/// The Tasks page.
pub(crate) struct TasksView {
    store: Entity<Store>,
    search: Entity<InputState>,
    new_title: Entity<InputState>,
    /// The New task field is open.
    adding: bool,
    scroll: ScrollHandle,
    /// The swipe over the rows.
    swipes: RowSwipes,
}

impl TasksView {
    /// The page over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a task: T-212 or its title"));
        cx.subscribe_in(&search, window, |this, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.store
                    .update(cx, |store, cx| store.search_tasks(&text, cx));
            }
        })
        .detach();
        let new_title = cx.new(|cx| InputState::new(window, cx).placeholder("What needs doing?"));
        cx.subscribe_in(
            &new_title,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add(window, cx);
                }
            },
        )
        .detach();
        Self {
            store,
            search,
            new_title,
            adding: false,
            scroll: ScrollHandle::new(),
            swipes: RowSwipes::default(),
        }
    }

    /// Adds the task typed in the New task field, and closes it.
    fn add(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let title = self.new_title.read(cx).value().to_string();
        self.store
            .update(cx, |store, cx| store.create_task(&title, cx));
        self.close_new(window, cx);
    }

    fn close_new(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.new_title
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.adding = false;
        cx.notify();
    }

    /// The head: Tasks, how many are open (and, with plans, how many run), New task.
    fn head(p: &Pal, count: usize, running: usize, cx: &Context<'_, Self>) -> Div {
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
                    .child("Tasks"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(if running > 0 {
                        format!("{count} open · {running} running")
                    } else {
                        format!("{count} open")
                    }),
            )
            .child(div().flex_1())
            .child(
                Button::new("new-task")
                    .ghost()
                    .small()
                    .icon(glyph(IconName::Plus, p.muted, 15.))
                    .tooltip("New task")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.adding = true;
                        let focus = this.new_title.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                        cx.notify();
                    })),
            )
    }

    /// The New task field, while it is open.
    fn new_task(&self, p: &Pal, hides_projects: bool, cx: &Context<'_, Self>) -> Div {
        let note = if hides_projects {
            "It's yours.".to_owned()
        } else {
            format!("It's yours. {APP_NAME} puts it in a project.")
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mx(px(8.))
            .mt(px(10.))
            .p(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("New task"),
            )
            .child(Input::new(&self.new_title))
            .child(div().text_size(px(11.5)).text_color(p.muted).child(note))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .justify_end()
                    .child(
                        Button::new("new-task-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_new(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("new-task-add")
                            .label("Add")
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
    }
}

impl Render for TasksView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let today = jiff::ZonedDateTime::now().date();
        let store_entity = self.store.clone();
        let store = self.store.read(cx);
        let hides_projects = store.hides_projects();
        let mut body = div()
            .flex()
            .flex_col()
            .px(px(16.))
            .py(px(4.))
            .max_w(px(980.));
        if self.adding {
            body = body.child(self.new_task(&p, hides_projects, cx));
        }
        body = body
            .child(div().px(px(8.)).pt(px(10.)).child(Input::new(&self.search)))
            .child(filter_bar::bar(&store_entity, &p, cx));
        let Some(state) = store.state() else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(Self::head(&p, 0, 0, cx))
                .child(
                    scroller("tasks-scroll", &self.scroll)
                        .child(body.child(loading(&p, "Loading…"))),
                );
        };
        let filter = store.task_filter();
        let list = TaskList::new(state, store.answers().gone(), &filter, today);
        // The sessions with no task go with the list only when nothing narrows it.
        let loose = if store.keeps_plans() && filter.is_empty() {
            loose_sessions::loose(state, jiff::Timestamp::now())
        } else {
            Loose::default()
        };
        let rows = Rows {
            store: &store_entity,
            read: store,
            state,
            p: &p,
            today,
            hides_projects,
            swipes: &self.swipes,
            cx,
        };
        let search = store.tasks().search();
        if search.is_on() {
            body = body.child(section_head(&p, "Tasks", None, false));
            body = rows.found(body, search.lookup().found(), search.text());
        } else {
            body = rows.list(body, &list, &loose, filter.is_empty());
            body = body.child(archived_fold(&store_entity, &rows, &p));
        }
        let count = list.len();
        let running = list.running.len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::head(&p, count, running, cx))
            .child(scroller("tasks-scroll", &self.scroll).child(body))
    }
}

/// What the rows are drawn with.
struct Rows<'a> {
    store: &'a Entity<Store>,
    read: &'a Store,
    state: &'a State,
    p: &'a Pal,
    today: Date,
    hides_projects: bool,
    swipes: &'a RowSwipes,
    cx: &'a App,
}

impl Rows<'_> {
    /// A row of `task`, and its subtasks under it when they are open.
    fn task(&self, mut column: Div, task: &Issue) -> Div {
        let project = if self.hides_projects {
            None
        } else {
            lookup::project_name(self.state, task.pid)
        };
        let open = self.read.tasks().shows_subtasks(task.id);
        let look = Look {
            project,
            today: self.today,
            health: self.state.health.as_ref(),
            open,
            nested: false,
            swipes: Some(self.swipes),
            ticked: self.read.is_ticked(task.id),
        };
        column = column.child(row::row(self.store, self.p, task, look, self.cx));
        if open {
            for subtask in list::subtasks(self.state, task.id, self.read.answers().gone()) {
                column = column.child(row::row(
                    self.store,
                    self.p,
                    subtask,
                    Look {
                        nested: true,
                        open: false,
                        ticked: false,
                        ..look
                    },
                    self.cx,
                ));
            }
        }
        column
    }

    /// The list's sections, the health card under Today, the empty state and what the filter
    /// hides; with plans, what runs and the sessions with no task (`loose`).
    fn list(&self, mut body: Div, list: &TaskList<'_>, loose: &Loose<'_>, unfiltered: bool) -> Div {
        let p = self.p;
        if !list.needs_you.is_empty() {
            body = body.child(section_head(
                p,
                "Needs you",
                Some(list.needs_you.len()),
                true,
            ));
            for task in &list.needs_you {
                body = self.task(body, task);
            }
        }
        for (title, tasks) in [("Running", &list.running), ("Blocked", &list.blocked)] {
            if !tasks.is_empty() {
                body = body.child(section_head(p, title, Some(tasks.len()), false));
                for task in tasks {
                    body = self.task(body, task);
                }
            }
        }
        let health = self.state.health.as_ref().filter(|_| unfiltered);
        if !list.today.is_empty() || health.is_some() {
            body = body.child(section_head(p, "Today", Some(list.today.len()), false));
            if let Some(health) = health {
                body = body.child(health_card(p, health));
            }
            for task in &list.today {
                body = self.task(body, task);
            }
        }
        if !list.rest.is_empty() {
            body = body.child(section_head(
                p,
                list.rest_heading(),
                Some(list.rest.len()),
                false,
            ));
            for task in &list.rest {
                body = self.task(body, task);
            }
        }
        if loose.len() > 0 {
            body = body.child(loose::section_head(p, loose.len()));
            body = body.children(
                loose
                    .waiting
                    .iter()
                    .chain(&loose.running)
                    .chain(&loose.recent)
                    .map(|session| self.session(session)),
            );
        }
        if list.is_empty() && loose.len() == 0 {
            body = body.child(
                div()
                    .px(px(8.))
                    .py(px(18.))
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child(list.empty_text()),
            );
        }
        if list.hidden > 0 {
            let store = self.store.clone();
            body = body.child(tab_stop(
                div()
                    .id("filter-show-all")
                    .border_1()
                    .rounded(px(6.))
                    .px(px(8.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .cursor_pointer()
                    .hover(|d| d.text_color(p.accent))
                    .child(format!(
                        "{} more hidden by the filter · Show all",
                        list.hidden
                    ))
                    .on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| {
                            store.set_task_filter(TaskFilter::default(), cx);
                        });
                    }),
                p,
            ));
        }
        body
    }

    /// A session with no task, as an item of the list: it opens the session.
    fn session(&self, session: &emberhut_api::work::RunnerSession) -> gpui_kit::AnyElement {
        loose::row(self.store, self.p, session)
    }

    /// Tasks found by the server: a spinner while they come, `No task matches …` when none do.
    fn found(&self, mut body: Div, found: Found<'_>, query: &str) -> Div {
        let p = self.p;
        match found {
            Found::Idle => body,
            Found::Loading => body.child(loading(p, "Looking…")),
            Found::Failed(reason) => body.child(note(p, reason.to_owned())),
            Found::Tasks(tasks) => {
                let shown: Vec<&Issue> = tasks
                    .iter()
                    .filter(|task| !self.read.is_gone(task.id))
                    .collect();
                if shown.is_empty() {
                    let text = if query.trim().is_empty() {
                        "Nothing here yet.".to_owned()
                    } else {
                        format!("No task matches “{}”.", query.trim())
                    };
                    return body.child(note(p, text));
                }
                for task in shown {
                    body = self.task(body, task);
                }
                body
            }
        }
    }
}

/// A section's title, its count beside it.
fn section_head(p: &Pal, title: &'static str, count: Option<usize>, accent: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .pt(px(16.))
        .pb(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if accent { p.accent } else { p.muted })
                .child(title),
        )
        .children(count.filter(|&count| count > 0).map(|count| {
            div()
                .text_size(px(11.5))
                .text_color(p.faint)
                .child(count.to_string())
        }))
}

/// A grey line in a list.
fn note(p: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .px(px(8.))
        .py(px(10.))
        .text_size(px(13.))
        .text_color(p.muted)
        .child(text.into())
}

/// A spinner and its words.
pub(crate) fn loading(p: &Pal, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(10.))
        .text_size(px(12.5))
        .text_color(p.muted)
        .child(Spinner::new().small().color(p.muted))
        .child(text)
}

/// Today from the phone's Health: steps toward the daily target, exercise minutes, and when
/// the phone last sent them.
fn health_card(p: &Pal, health: &HealthToday) -> Div {
    let steps = health.steps.unwrap_or(0.0);
    let sent = health.at.as_ref().and_then(clock::instant).map(|at| {
        format!(
            "Today from Apple Health · sent {}",
            clock::relative(at, jiff::Timestamp::now())
        )
    });
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .mx(px(8.))
        .mb(px(6.))
        .p(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .child(glyph(IconName::Footprints, p.success, 14.))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{} steps", thousands(steps))),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(format!("of {}", thousands(health.steps_target))),
                )
                .child(div().flex_1())
                .child(div().text_size(px(12.)).text_color(p.muted).child(
                    health.exercise_min.map_or_else(
                        || "No exercise minutes".to_owned(),
                        |minutes| format!("{} min exercise", thousands(minutes)),
                    ),
                )),
        )
        .child(progress(p, steps / health.steps_target.max(1.0), p.success))
        .children(sent.map(|sent| div().text_size(px(11.)).text_color(p.faint).child(sent)))
}

/// The folded `Archived tasks` row at the foot; open, the archived tasks newest first.
fn archived_fold(store: &Entity<Store>, rows: &Rows<'_>, p: &Pal) -> Div {
    let archived = rows.read.tasks().archived();
    let toggle = store.clone();
    let chevron = if archived.is_some() {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let mut fold = div().flex().flex_col().pt(px(14.)).child(tab_stop(
        div()
            .id("archived-tasks")
            .border_1()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .text_size(px(12.5))
            .text_color(p.muted)
            .child(glyph(chevron, p.muted, 13.))
            .child("Archived tasks")
            .on_click(move |_, _, cx| toggle.update(cx, Store::toggle_archived_tasks)),
        p,
    ));
    if let Some(archived) = archived {
        fold = rows.found(fold, archived.found(), "");
    }
    fold
}
