//! The spotlight over the window (⌘K, or Search in the chat's bar): the window dimmed, a card
//! near its top with the field focused, and what it finds under it: projects, people, then
//! tasks (the latest with nothing typed). ↑ and ↓ move the pick (kept in view), Return opens
//! it, the ✕ clears the field; Esc or a click outside closes it.

use emberhut_api::facts::Fact;
use emberhut_api::tasks::{Issue, TaskStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, EventEmitter, FocusHandle, FontWeight, Role, ScrollHandle,
    Stateful, Window, actions, div, px,
};
use jiff::ZonedDateTime;

use super::a11y::{self, Accessible as _};
use super::palette::{Pal, pal, room_color};
use super::tasks::marks::{deadline_chip, glyph, status_icon};
use super::window::MainWindow;
use crate::prose::direction::marked;
use crate::store::spotlight::{Found, Results};
use crate::store::tasks::{counts, words};
use crate::store::{Page, Store, lookup};

actions!(
    spotlight,
    [
        /// Opens the spotlight over the window.
        OpenSpotlight,
    ]
);

/// The card's width.
const WIDTH: f32 = 620.;

/// The card's height at most.
const HEIGHT: f32 = 460.;

/// How far from the window's top the card sits.
const TOP: f32 = 70.;

/// What the spotlight asks of its window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SpotlightEvent {
    /// Close it.
    Close,
    /// Open what was picked (and close it).
    Open(Found),
}

/// The spotlight's card.
pub(crate) struct SpotlightView {
    store: Entity<Store>,
    field: Entity<InputState>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    /// The pick the list was last scrolled to, so a new pick is brought into view once.
    shown_pick: Option<usize>,
}

impl EventEmitter<SpotlightEvent> for SpotlightView {}

impl SpotlightView {
    /// The card over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let field = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(
            &field,
            window,
            |this, input, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.store
                        .update(cx, |store, cx| store.spotlight_type(&text, cx));
                }
                InputEvent::PressEnter { .. } => this.open_picked(cx),
                _ => {}
            },
        )
        .detach();
        Self {
            store,
            field,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            shown_pick: None,
        }
    }

    /// The spotlight comes up: the field empties, takes the keyboard, and the latest tasks are
    /// asked for.
    pub(crate) fn show(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let placeholder = if self.store.read(cx).hides_projects() {
            "Search tasks and people"
        } else {
            "Search tasks, projects and people"
        };
        self.field.update(cx, |field, cx| {
            field.set_value("", window, cx);
            field.set_placeholder(placeholder, window, cx);
            field.focus(window, cx);
        });
        self.store.update(cx, Store::open_spotlight);
        cx.notify();
    }

    /// Opens the picked result.
    fn open_picked(&mut self, cx: &mut Context<'_, Self>) {
        let store = self.store.read(cx);
        let results = store
            .spotlight()
            .results(store.state(), store.places().people());
        if let Some(found) = results.order.get(results.selected).cloned() {
            cx.emit(SpotlightEvent::Open(found));
        }
    }

    /// Moves the pick and keeps it in view.
    fn step(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        self.store
            .update(cx, |store, cx| store.spotlight_step(step, cx));
        cx.notify();
    }

    /// Clears the field.
    fn clear(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.field
            .update(cx, |field, cx| field.set_value("", window, cx));
        self.store
            .update(cx, |store, cx| store.spotlight_type("", cx));
    }
}

/// A section's title over its results, with its empty line.
fn section(p: &Pal, title: &'static str, empty: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(16.))
        .pt(px(10.))
        .pb(px(4.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title.to_uppercase()),
        )
        .children(empty.map(|empty| div().text_size(px(13.)).text_color(p.muted).child(empty)))
}

/// A result's row: lit when picked; a click opens it.
fn row(
    p: &Pal,
    index: usize,
    picked: bool,
    found: Found,
    cx: &Context<'_, SpotlightView>,
) -> Stateful<Div> {
    div()
        .id(ElementId::Name(format!("spotlight-{index}").into()))
        // A result, the picked one selected; its caller names it.
        .role(Role::ListBoxOption)
        .aria_selected(picked)
        .flex()
        .items_center()
        .gap(px(8.))
        .mx(px(4.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(picked, |d| d.bg(p.accent.opacity(0.16)))
        .when(!picked, |d| d.hover(|d| d.bg(p.hover)))
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(SpotlightEvent::Open(found.clone()))))
}

/// A task in the results: its state, its title, its key, project and status, its deadline.
fn task_hit(p: &Pal, store: &Store, task: &Issue, today: jiff::civil::Date) -> Vec<AnyElement> {
    let project = store
        .state()
        .and_then(|state| lookup::project_name(state, task.pid));
    let line = [
        Some(task.key.clone()),
        project.map(str::to_owned),
        Some(words::name(task)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let day = (task.status != TaskStatus::Done)
        .then(|| task.deadline.as_ref().or(task.due.as_ref()))
        .flatten();
    let hard = task.hard_deadline || task.hard;
    vec![
        status_icon(p, task).into_any_element(),
        div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .flex_1()
            .min_w_0()
            .child(div().truncate().child(marked(&task.title)))
            .child(
                div()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(line),
            )
            .into_any_element(),
        div()
            .children(counts::deadline_chip(day, hard, today).map(|chip| deadline_chip(p, chip)))
            .into_any_element(),
    ]
}

/// A person's result: their name, and what is known of them under it.
fn person_lines(p: &Pal, person: &Fact) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(div().truncate().child(marked(&person.title)))
        .when(!person.text.trim().is_empty(), |d| {
            d.child(
                div()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(person.text.clone()),
            )
        })
}

impl SpotlightView {
    /// The results, as rows under their sections, and the place among them of the picked row.
    fn results(
        &self,
        p: &Pal,
        results: &Results<'_>,
        store: &Store,
        cx: &Context<'_, Self>,
    ) -> (Stateful<Div>, Option<usize>) {
        let mut list = div()
            .id("spotlight-results")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .py(px(6.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        // Each result's number in the order shown, and each child's place in the list (the
        // sections' titles are children too), to scroll the picked one into view.
        let (mut index, mut children, mut picked_child) = (0, 0, None);
        let mut next = |found: Found, children: usize| {
            let at = index;
            index += 1;
            let picked = at == results.selected;
            if picked {
                picked_child = Some(children);
            }
            (at, picked, found)
        };
        if !results.projects.is_empty() {
            list = list.child(section(p, "Projects", None));
            children += 1;
            for project in &results.projects {
                let (at, picked, found) = next(Found::Project(project.id), children);
                children += 1;
                let goal = store
                    .state()
                    .and_then(|state| lookup::goal_rank(state, project.goal));
                let note = project_note(project);
                list = list.child(
                    row(p, at, picked, found, cx)
                        .aria_label(a11y::line(&["Project", &project.name, &note]))
                        .child(glyph(IconName::Folder, room_color(goal, p), 15.))
                        .child(div().flex_1().truncate().child(marked(&project.name)))
                        .child(div().text_size(px(11.5)).text_color(p.muted).child(note)),
                );
            }
        }
        if !results.people.is_empty() {
            list = list.child(section(p, "People", None));
            children += 1;
            for person in &results.people {
                let (at, picked, found) = next(Found::Person(person.id.clone()), children);
                children += 1;
                list = list.child(
                    row(p, at, picked, found, cx)
                        .aria_label(a11y::line(&["Person", &person.title]))
                        .child(glyph(IconName::CircleUser, p.info, 16.))
                        .child(person_lines(p, person)),
                );
            }
        }
        let blank = store.spotlight().is_blank();
        let empty = tasks_empty(results, store);
        list = list.child(section(
            p,
            if blank { "Latest tasks" } else { "Tasks" },
            empty,
        ));
        children += 1;
        match &results.tasks {
            None => {
                list = list.child(div().flex().justify_center().p(px(10.)).child(glyph(
                    IconName::LoaderCircle,
                    p.muted,
                    16.,
                )));
            }
            Some(tasks) => {
                let today = ZonedDateTime::now().date();
                for task in tasks {
                    let (at, picked, found) = next(Found::Task(task.id), children);
                    children += 1;
                    list = list.child(
                        row(p, at, picked, found, cx)
                            .aria_label(a11y::line(&[&task.title, &task.key, &words::name(task)]))
                            .children(task_hit(p, store, task, today)),
                    );
                }
            }
        }
        (list, picked_child)
    }
}

impl Render for SpotlightView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let store = self.store.read(cx);
        let results = store
            .spotlight()
            .results(store.state(), store.places().people());
        let typed = !store.spotlight().text().is_empty();
        let (list, picked) = self.results(&p, &results, store, cx);
        if picked != self.shown_pick {
            self.shown_pick = picked;
            if let Some(child) = picked {
                self.scroll.scroll_to_item(child);
            }
        }
        let field = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(12.))
            .border_b_1()
            .border_color(p.border)
            .child(glyph(IconName::Search, p.muted, 18.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(16.))
                    .child(Input::new(&self.field).appearance(false)),
            )
            .when(typed, |d| {
                d.child(
                    div()
                        .id("spotlight-clear")
                        .button_named("Clear the search")
                        .cursor_pointer()
                        .child(glyph(IconName::CircleX, p.muted, 16.))
                        .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx))),
                )
            });
        div()
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .bg(gpui_kit::black().opacity(0.12))
            // What is under the dimmed backdrop takes no click or hover while it shows.
            .occlude()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(SpotlightEvent::Close)),
            )
            .child(
                div()
                    .id("spotlight")
                    .read_as(Role::Dialog, "Search")
                    .track_focus(&self.focus)
                    .key_context("Spotlight")
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                        cx.stop_propagation();
                        this.step(-1, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                        cx.stop_propagation();
                        this.step(1, cx);
                    }))
                    .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SpotlightEvent::Close);
                    }))
                    .mt(px(TOP))
                    .w(px(WIDTH))
                    .max_h(px(HEIGHT))
                    .flex()
                    .flex_col()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface_raised)
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(field)
                    .child(list),
            )
    }
}

/// The chat bar's Search: the spotlight, and its shortcut.
pub(crate) fn search_button(p: &Pal) -> Stateful<Div> {
    let keys = if cfg!(target_os = "macos") {
        "⌘K"
    } else {
        "Ctrl+K"
    };
    let tip = format!("Find a task, project or person ({keys})");
    div()
        .id("search")
        .button_named("Search")
        .hint(tip.clone())
        .accessibility_id("search")
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(9.))
        .py(px(4.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .text_size(px(12.5))
        .child(glyph(IconName::Search, p.muted, 13.))
        .child("Search")
        .child(div().text_size(px(11.)).text_color(p.faint).child(keys))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
        .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSpotlight), cx))
}

impl MainWindow {
    /// Brings the spotlight up over the window.
    pub(crate) fn open_spotlight(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.spotlight_open = true;
        self.spotlight
            .update(cx, |spotlight, cx| spotlight.show(window, cx));
        cx.notify();
    }

    /// What the spotlight asked: closed, or a pick to open beside the chat (a task over what
    /// the column shows, Back returning there; a project or a person on a page of its own).
    pub(super) fn spotlight_asked(
        &mut self,
        _: &Entity<SpotlightView>,
        event: &SpotlightEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.spotlight_open = false;
        match event {
            SpotlightEvent::Close => {}
            SpotlightEvent::Open(Found::Task(task)) => self.push(Page::Task(*task), window, cx),
            SpotlightEvent::Open(Found::Project(project)) => {
                self.open(Page::Project(*project), window, cx);
            }
            SpotlightEvent::Open(Found::Person(person)) => {
                let key = self.store.update(cx, |store, _| store.person_key(person));
                self.open(Page::Person(key), window, cx);
            }
        }
        self.keep_focus(window, cx);
        cx.notify();
    }
}

/// A project's note in the results: Archived, Paused, or how many are open.
fn project_note(project: &emberhut_api::tasks::Project) -> String {
    match project.status {
        emberhut_api::tasks::ProjectStatus::Archived => "Archived".to_owned(),
        emberhut_api::tasks::ProjectStatus::Paused => "Paused".to_owned(),
        _ => format!("{} open", project.open),
    }
}

/// The tasks section's line when it has none: why they didn't come, `No tasks yet.`, or
/// `No task matches “x”.`.
fn tasks_empty(results: &Results<'_>, store: &Store) -> Option<String> {
    match (&results.tasks, results.failed) {
        (_, Some(reason)) => Some(reason.to_owned()),
        (Some(tasks), None) if tasks.is_empty() => Some(if store.spotlight().is_blank() {
            "No tasks yet.".to_owned()
        } else {
            format!("No task matches “{}”.", store.spotlight().text().trim())
        }),
        _ => None,
    }
}
