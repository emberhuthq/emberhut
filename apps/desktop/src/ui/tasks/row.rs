//! A task's row in a list: its state icon (or, for the user's own to-do, a tick to mark it
//! done), its title and a grey line of what says something, a walk task's minutes today, a
//! progress bar, a parent's subtask count that opens them under it, and its deadline chip. A
//! click opens its page in the column; a right-click offers Mark done, Not now, Archive (or
//! Unarchive).
//!
//! On a server that keeps plans, a running task's row is drawn as running (a live dot, the step
//! it is on or what blocks it, its n/m ticks and a thin bar), and an open one says its ticks, or
//! "No plan yet", quietly at its end.

use emberhut_api::health_today::HealthToday;
use emberhut_api::tasks::{Issue, TaskStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ElementId, Entity, FontWeight, div, px};
use jiff::civil::Date;

use super::marks::{deadline_chip, glyph, progress, status_icon, thousands};
use super::plan_marks;
use super::swipe::{RowSwipes, Swiped};
use crate::prose::direction;
use crate::store::lookup;
use crate::store::tasks::counts;
use crate::store::tasks::plan::{self, RowPlan, Tone};
use crate::store::tasks::words;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;

/// How a row is drawn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Look<'a> {
    /// Its project's name, when the list names it.
    pub(crate) project: Option<&'a str>,
    /// Today, for its deadline chip.
    pub(crate) today: Date,
    /// Today's health, for a walk task's line.
    pub(crate) health: Option<&'a HealthToday>,
    /// Whether its subtasks show under it (a parent's chip points up).
    pub(crate) open: bool,
    /// Under its parent: indented, without a tick or a subtask chip.
    pub(crate) nested: bool,
    /// The list's swipes, when its rows take them.
    pub(crate) swipes: Option<&'a RowSwipes>,
    /// Its tick was just pressed: the check shows ([`Store::is_ticked`]).
    pub(crate) ticked: bool,
}

/// Whether the user ticks `task` done right on its row: their own to-do with nothing under it.
#[must_use]
pub(crate) fn tickable(task: &Issue) -> bool {
    !lookup::is_assistants(task) && task.status == TaskStatus::Todo && task.sub.is_none()
}

/// Whether Not now puts `task` off: a to-do or one waiting on the user.
#[must_use]
pub(crate) fn parkable(task: &Issue) -> bool {
    matches!(task.status, TaskStatus::Todo | TaskStatus::Waiting)
}

/// The row.
pub(crate) fn row(
    store: &Entity<Store>,
    p: &Pal,
    task: &Issue,
    look: Look<'_>,
    cx: &App,
) -> AnyElement {
    let id = task.id;
    let planned = store
        .read(cx)
        .keeps_plans()
        .then(|| plan::row_plan(task))
        .flatten();
    let body = match &planned {
        Some(planned) => planned_body(p, task, planned, quiet_of(store, task, cx).as_deref()),
        None => row_body(p, task, look),
    };
    let lead = lead_of(store, p, task, look, planned.as_ref(), cx);
    let chip = counts::deadline_chip(task.deadline.as_ref(), task.hard_deadline, look.today);
    let subtasks = task.sub.as_ref().filter(|_| !look.nested).map(|rollup| {
        let store = store.clone();
        let chevron = if look.open {
            IconName::ChevronUp
        } else {
            IconName::ChevronDown
        };
        tab_stop(
            div()
                .id(ElementId::Name(format!("subtasks-{id}").into()))
                .button_named(if look.open {
                    "Hide subtasks".to_owned()
                } else {
                    format!("Show {} subtasks", rollup.total)
                })
                .aria_expanded(look.open)
                .border_1()
                .flex()
                .items_center()
                .gap(px(3.))
                .px(px(8.))
                .py(px(3.))
                .rounded_full()
                .bg(p.hover)
                .cursor_pointer()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(rollup.total.to_string())
                .child(glyph(chevron, p.muted, 11.))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    store.update(cx, |store, cx| store.toggle_subtasks(id, cx));
                }),
            p,
        )
    });
    let spoken = a11y::line(&[
        &task.title,
        &words::name(task),
        &counts::row_line(task, look.project),
        &planned.as_ref().map_or_default(plan::spoken_row),
    ]);
    let row = div()
        .id(ElementId::Name(format!("task-{id}").into()))
        .button_named(spoken)
        .hint("Shows its details")
        .border_1()
        .flex()
        .items_start()
        .gap(px(10.))
        .px(px(8.))
        .py(px(7.))
        .when(look.nested, |d| d.pl(px(34.)))
        .rounded(px(7.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(div().pt(px(1.)).child(lead))
        .child(body)
        .children(subtasks)
        .children(chip.map(|chip| deadline_chip(p, chip)))
        // Over the list or the page it is on, in the window's column: Back returns there.
        .on_click(move |_, window, cx| column::push(Page::Task(id), window, cx));
    let row = tab_stop(row, p);
    let row = match look.swipes {
        Some(swipes) => with_swipes(row, store, task, swipes.clone()),
        None => row,
    };
    with_menu(row, store, task).into_any_element()
}

/// A row's text: its title, its grey line, a walk task's minutes today and its progress bar.
fn row_body(p: &Pal, task: &Issue, look: Look<'_>) -> gpui_kit::Div {
    let id = task.id;
    let title = div()
        .truncate()
        .text_size(px(13.))
        // Its direction mark first, so a mostly Arabic title reads in its own order.
        .child(direction::marked(&task.title));
    let mut body = div().flex().flex_col().flex_1().min_w_0().child(title);
    let line = counts::row_line(task, look.project);
    if !line.is_empty() {
        body = body.child(
            div()
                .truncate()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(line),
        );
    }
    if task.status != TaskStatus::Done
        && let Some(health) = look.health
        && let Some(target) = health.walk_target(id)
    {
        let minutes = health.move_min.unwrap_or(0.0);
        body = body
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.success)
                    .child(format!(
                        "Today: {} of {} min · {} steps",
                        thousands(minutes),
                        thousands(target),
                        thousands(health.steps.unwrap_or(0.0))
                    )),
            )
            .child(progress(p, minutes / target.max(1.0), p.success));
    }
    if counts::shows_bar(task)
        && let Some(fraction) = counts::fraction(task)
    {
        let fill = if counts::subtask_needs_you(task) {
            p.accent
        } else {
            p.info
        };
        body = body.child(progress(p, fraction, fill));
    }
    body
}

/// The mark a row leads with: its plan's (running, blocked, waiting on the user), else the tick
/// of the user's own to-do, else its state icon.
fn lead_of(
    store: &Entity<Store>,
    p: &Pal,
    task: &Issue,
    look: Look<'_>,
    planned: Option<&RowPlan>,
    cx: &App,
) -> AnyElement {
    let id = task.id;
    let mark = planned.and_then(|planned| {
        plan_marks::lead(
            p,
            planned.lead,
            ElementId::Name(format!("task-lead-{id}").into()),
            cx,
        )
    });
    if let Some(mark) = mark {
        mark
    } else if tickable(task) && !look.nested {
        tab_stop(
            super::tick::tick(store, p, id, &task.title, 17., look.ticked).border_1(),
            p,
        )
        .into_any_element()
    } else {
        status_icon(p, task).into_any_element()
    }
}

/// How long `task` has gone quiet, when it runs and nothing moved for a while.
fn quiet_of(store: &Entity<Store>, task: &Issue, cx: &App) -> Option<String> {
    let state = store.read(cx).state()?;
    let sessions = crate::store::sessions::lists::all(state);
    plan::quiet(task, &sessions, jiff::Timestamp::now())
}

/// A row's text with its plan: its title; its one line (the step it is on, what it waits on or
/// what blocks it) with `quiet 25m` at its end when it has gone quiet; then its bar with its
/// n/m beside it, so the title keeps the width.
fn planned_body(p: &Pal, task: &Issue, planned: &RowPlan, quiet: Option<&str>) -> gpui_kit::Div {
    let (text, tone) = &planned.line;
    let color = match tone {
        Tone::Muted => p.muted,
        Tone::Faint => p.faint,
        Tone::Danger => p.danger,
        Tone::Accent => p.accent,
    };
    let line = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.5))
                .text_color(color)
                .when(*tone == Tone::Accent, |d| {
                    d.font_weight(FontWeight::SEMIBOLD)
                })
                .child(text.clone()),
        )
        .children(quiet.map(|quiet| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(3.))
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(glyph(IconName::Clock, p.muted, 11.))
                .child(quiet.to_owned())
        }));
    let bar = planned.bar.as_ref().map(|bar| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .pt(px(5.))
            .child(div().flex_1().child(plan_marks::bar(p, bar, 4.)))
            .children(planned.ticks.clone().map(|ticks| {
                div()
                    .flex_none()
                    .min_w(px(28.))
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(ticks)
            }))
    });
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .flex_1()
        .min_w_0()
        .child(
            div()
                .truncate()
                .text_size(px(13.))
                // Its direction mark first, so a mostly Arabic title reads in its own order.
                .child(direction::marked(&task.title)),
        )
        .child(line)
        .children(bar)
}

/// `row` taking two-finger swipes: right marks the user's own task done; left puts it off a
/// week (or, when it can't be, archives it, or brings an archived one back).
fn with_swipes(
    row: gpui_kit::Stateful<gpui_kit::Div>,
    store: &Entity<Store>,
    task: &Issue,
    swipes: RowSwipes,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let id = task.id;
    let (tick, park, open, archived) = (
        tickable(task),
        parkable(task),
        task.is_open(),
        task.status == TaskStatus::Canceled,
    );
    let store = store.clone();
    row.on_scroll(move |event, _, cx| {
        let Some(way) = swipes.feed(id, event) else {
            return;
        };
        store.update(cx, |store, cx| match way {
            Swiped::Right if tick => store.tick_done(id, cx),
            Swiped::Left if park => store.not_now(id, cx),
            Swiped::Left if open => store.archive(id, cx),
            Swiped::Left if archived => store.unarchive(id, cx),
            Swiped::Right | Swiped::Left => {}
        });
    })
}

/// `row` with its right-click menu: what can be done to `task` from a list.
fn with_menu(
    row: gpui_kit::Stateful<gpui_kit::Div>,
    store: &Entity<Store>,
    task: &Issue,
) -> impl IntoElement {
    let id = task.id;
    let (tick, park, open, archived) = (
        tickable(task),
        parkable(task),
        task.is_open(),
        task.status == TaskStatus::Canceled,
    );
    let store = store.clone();
    row.context_menu(move |mut menu, _, _| {
        let on = |action: fn(&mut Store, emberhut_api::ids::TaskId, &mut Context<'_, Store>)| {
            let store = store.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut App| {
                store.update(cx, |store, cx| action(store, id, cx));
            }
        };
        if tick {
            menu = menu.item(PopupMenuItem::new("Mark done").on_click(on(Store::mark_done)));
        }
        if park {
            menu = menu.item(PopupMenuItem::new("Not now").on_click(on(Store::not_now)));
        }
        if open {
            menu = menu.item(PopupMenuItem::new("Archive").on_click(on(Store::archive)));
        }
        if archived {
            menu = menu.item(PopupMenuItem::new("Unarchive").on_click(on(Store::unarchive)));
        }
        menu
    })
}
