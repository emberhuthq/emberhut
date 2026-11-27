//! The rest of a task's page, folded: what happened since the ask, the details (where it
//! belongs, its count and owner, the day to decide an ask by, its deadline, its size, ultra
//! mode, the work linked to it), the user's notes or the worker's brief (edited in place), its
//! history, and Stop on a working task (asked first).

use emberhut_api::task_actions::{Edit, TaskChange};
use emberhut_api::task_page::{LinkKind, TaskLink};
use emberhut_api::tasks::TaskStatus;
use gpui_kit::Focusable as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::date_picker::DatePicker;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, FontWeight, SharedString, div, px};
use jiff::ToSpan as _;

use super::reply::who;
use super::text::{Long, collapsible, link_button};
use super::{Part, TaskView, day, fold, fold_row, section};
use crate::brand::APP_NAME;
use crate::store::tasks::filter::When;
use crate::store::tasks::{clock, counts, history};
use crate::store::{Page, lookup};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// A label and its value, one line of the details.
fn line(p: &Pal, label: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(4.))
        .text_size(px(13.))
        .child(
            div()
                .w(px(96.))
                .flex_none()
                .text_color(p.muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value))
}

/// A step's time: `Thu 9:12 AM`.
fn step_time(at: &emberhut_api::time::Timestamp, now: &jiff::ZonedDateTime) -> String {
    clock::instant(at).map_or_else(String::new, |at| clock::day_and_time(at, now))
}

/// The steps since the assistant asked, with their count, folded: the last four steps after the ask, so an outdated ask shows
/// itself.
pub(super) fn since_asked(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let (Some(decision), Some(page)) = (
        part.task
            .decision
            .as_ref()
            .filter(|_| part.task.status == TaskStatus::Waiting),
        part.page,
    ) else {
        return body;
    };
    let count = history::since_asked_count(&page.history, decision);
    if count == 0 {
        return body;
    }
    let p = part.p;
    body = body.child(
        fold_row(
            p,
            "since-asked",
            format!("Since {APP_NAME} asked · {count}"),
            view.is_open(fold::SINCE),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_open(fold::SINCE);
            cx.notify();
        })),
    );
    if view.is_open(fold::SINCE) {
        for (i, moment) in history::since_asked(&page.history, decision)
            .into_iter()
            .enumerate()
        {
            let key = format!("since-{i}");
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .pb(px(6.))
                    .child(collapsible(
                        p,
                        part.store,
                        Long {
                            key: &key,
                            text: &moment.text,
                            lines: 3,
                            title: None,
                        },
                        view,
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.faint)
                            .child(step_time(&moment.at, part.now)),
                    ),
            );
        }
    }
    body
}

/// The folded details.
pub(super) fn details(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let (p, task) = (part.p, part.task);
    body = body.child(
        fold_row(
            p,
            "details",
            "Details".to_owned(),
            view.is_open(fold::DETAILS),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_open(fold::DETAILS);
            cx.notify();
        })),
    );
    if !view.is_open(fold::DETAILS) {
        return body;
    }
    let state = part.read.state();
    if let Some(parent) = task.parent {
        let name = part.read.task_of(parent).map_or_else(
            || part.read.task_key(parent),
            |t| format!("{} {}", t.key, t.title),
        );
        body = body.child(line(
            p,
            "Part of",
            link_button(p, "part-of".into(), name).on_click(move |_, window, cx| {
                column::push(Page::Task(parent), window, cx);
            }),
        ));
    }
    if let Some(count) = counts::count_text(task) {
        body = body.child(line(p, "Progress", count));
    }
    let owner = if lookup::is_assistants(task) {
        APP_NAME
    } else {
        "You"
    };
    body = body.child(line(p, "Owner", owner));
    let hides_goals = state
        .and_then(|state| state.ui.as_ref())
        .is_none_or(|ui| ui.hide_goals);
    if !hides_goals
        && let Some(goal) =
            state.and_then(|state| state.goals.iter().find(|goal| Some(goal.id) == task.goal))
    {
        body = body.child(line(p, "Goal", goal.title.clone()));
    }
    if !part.read.hides_projects()
        && let Some(project) = state.and_then(|state| lookup::project_name(state, task.pid))
    {
        body = body.child(line(p, "Project", project.to_owned()));
    }
    if task.status == TaskStatus::Waiting {
        body = decide_by(body, part, view);
    }
    if task.status != TaskStatus::Done {
        body = deadline(body, part, view);
        body = size(body, part);
        if lookup::is_assistants(task) {
            body = ultra(body, part, view, cx);
        }
    }
    for (i, link) in part
        .page
        .map_or(&[][..], |page| &page.links)
        .iter()
        .enumerate()
    {
        body = body.child(link_row(p, i, link, part.now));
    }
    body
}

/// The day to decide an ask by: a switch (a week out when turned on), a date, and which group
/// of Needs you it sits in.
fn decide_by(mut body: Div, part: &Part<'_>, view: &TaskView) -> Div {
    let (p, task) = (part.p, part.task);
    let (store, id) = (part.store.clone(), task.id);
    let today = part.now.date();
    body = body.child(line(
        p,
        "Decide by",
        Switch::new("decide-by")
            .checked(task.decide_by.is_some())
            .on_click(move |on, _, cx| {
                let set = if *on {
                    Edit::Set(day(today.checked_add(6.days()).unwrap_or(today)))
                } else {
                    Edit::Clear
                };
                let change = TaskChange {
                    decide_by: set,
                    ..TaskChange::of(id)
                };
                store.update(cx, |store, cx| store.change_task(change, cx));
            }),
    ));
    if task.decide_by.is_some() {
        body = body.child(line(
            p,
            "Date",
            DatePicker::new(&view.decide_picker).small(),
        ));
    }
    let when = task
        .decide_by
        .as_ref()
        .map_or(When::ThisWeek, |day| When::of(day, today));
    body.child(
        div()
            .text_size(px(11.5))
            .text_color(p.muted)
            .pb(px(4.))
            .child(format!(
                "It sits under {} in Needs you. More than a month out, it waits folded and isn't counted.",
                when.name()
            )),
    )
}

/// Its own deadline: a switch (a week out when turned on), a date, and whether it is hard; with
/// none of its own, the project's.
fn deadline(mut body: Div, part: &Part<'_>, view: &TaskView) -> Div {
    let (p, task) = (part.p, part.task);
    let (store, id) = (part.store.clone(), task.id);
    let today = part.now.date();
    let hard = task.hard;
    body = body.child(line(
        p,
        "Deadline",
        Switch::new("deadline")
            .checked(task.due.is_some())
            .on_click(move |on, _, cx| {
                let set = if *on {
                    Edit::Set(day(today.checked_add(7.days()).unwrap_or(today)))
                } else {
                    Edit::Clear
                };
                let change = TaskChange {
                    deadline: set,
                    hard: Some(hard),
                    ..TaskChange::of(id)
                };
                store.update(cx, |store, cx| store.change_task(change, cx));
            }),
    ));
    if task.due.is_some() {
        let store = part.store.clone();
        let own = task.due.clone();
        body = body
            .child(line(
                p,
                "Date",
                DatePicker::new(&view.deadline_picker).small(),
            ))
            .child(line(
                p,
                "Hard deadline",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        Switch::new("hard")
                            .checked(hard)
                            .on_click(move |on, _, cx| {
                                let change = TaskChange {
                                    deadline: own.clone().map_or(Edit::Keep, Edit::Set),
                                    hard: Some(*on),
                                    ..TaskChange::of(id)
                                };
                                store.update(cx, |store, cx| store.change_task(change, cx));
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.muted)
                            .child("Someone else holds you to it: a client, an exam."),
                    ),
            ));
    } else if let Some(inherited) = &task.deadline {
        let date = lookup::civil_day(inherited.as_str()).map_or_else(
            || inherited.as_str().to_owned(),
            |date| date.strftime("%-d %b").to_string(),
        );
        let hard = if task.hard_deadline { ", hard" } else { "" };
        body = body.child(
            div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .pb(px(4.))
                .child(format!("From the project: {date}{hard}")),
        );
    }
    body
}

/// Its size: Not sized, S about an hour, M about 3 hours, L about a day, XL several days.
fn size(body: Div, part: &Part<'_>) -> Div {
    let (p, task) = (part.p, part.task);
    let shown: SharedString = task
        .size
        .as_ref()
        .map_or_else(|| "Not sized".to_owned(), counts::size_label)
        .into();
    let (store, id) = (part.store.clone(), task.id);
    body.child(line(
        p,
        "Size",
        Button::new("size")
            .label(shown)
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                let pick = |set: Edit<emberhut_api::tasks::TaskSize>| {
                    let store = store.clone();
                    move |_: &gpui_kit::ClickEvent,
                          _: &mut gpui_kit::Window,
                          cx: &mut gpui_kit::App| {
                        let change = TaskChange {
                            size: set.clone(),
                            ..TaskChange::of(id)
                        };
                        store.update(cx, |store, cx| store.change_task(change, cx));
                    }
                };
                menu = menu.item(PopupMenuItem::new("Not sized").on_click(pick(Edit::Clear)));
                for size in counts::SIZES {
                    menu = menu.item(
                        PopupMenuItem::new(counts::size_label(&size))
                            .on_click(pick(Edit::Set(size))),
                    );
                }
                menu
            }),
    ))
}

/// Ultra mode on the assistant's tasks: its next run splits the work across many agents.
fn ultra(body: Div, part: &Part<'_>, view: &TaskView, cx: &Context<'_, TaskView>) -> Div {
    let (p, task) = (part.p, part.task);
    let on = view.ultra.unwrap_or(task.ultra);
    let id = task.id;
    body.child(line(
        p,
        "Ultra mode",
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(Switch::new("ultra").checked(on).on_click(cx.listener(
                move |this, on: &bool, _, cx| {
                    this.ultra = Some(*on);
                    let change = TaskChange {
                        ultra: Some(*on),
                        ..TaskChange::of(id)
                    };
                    this.store
                        .update(cx, |store, cx| store.change_task(change, cx));
                    cx.notify();
                },
            )))
            .child(div().text_size(px(11.5)).text_color(p.muted).child(
                "Its next run splits the work across many agents in parallel, each part checked. \
                 It posts a plan and estimate first; a big one waits for your OK.",
            )),
    ))
}

/// A piece of work that moves the task: its icon, title, where it stands, its machine or key and
/// time, its latest line.
fn link_row(
    p: &Pal,
    i: usize,
    link: &TaskLink,
    now: &jiff::ZonedDateTime,
) -> gpui_kit::Stateful<Div> {
    let icon = match link.kind {
        LinkKind::Thread => IconName::MessagesSquare,
        LinkKind::Ticket => IconName::Ticket,
        LinkKind::Chat => IconName::MessageCircle,
        _ => IconName::Link,
    };
    let status = link.status.as_deref().filter(|status| *status != "unknown");
    let status_word = status.map(|status| {
        let color = match status {
            "completed" | "Done" => p.success,
            "failed" | "interrupted" | "Stuck" => p.danger,
            "waiting" | "Waiting" => p.accent,
            _ => p.warn,
        };
        let word = if status == "completed" {
            "finished".to_owned()
        } else {
            status.to_lowercase()
        };
        div()
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .child(word)
    });
    let title = if link.title.is_empty() {
        link.reference.clone()
    } else {
        link.title.clone()
    };
    let under = [
        match link.kind {
            LinkKind::Thread => link.machine.as_ref().map(|m| m.as_str().to_owned()),
            LinkKind::Ticket if link.title != link.reference => Some(link.reference.clone()),
            _ => None,
        },
        link.at.as_ref().map(|at| step_time(at, now)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    div()
        .id(ElementId::Name(format!("link-{i}").into()))
        .flex()
        .items_start()
        .gap(px(10.))
        .py(px(5.))
        .child(div().pt(px(2.)).child(glyph(icon, p.muted, 15.)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .child(div().truncate().text_size(px(13.)).child(title))
                        .children(status_word),
                )
                .when(!under.is_empty(), |d| {
                    d.child(div().text_size(px(11.)).text_color(p.faint).child(under))
                })
                .children(link.line.clone().map(|line| {
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .line_clamp(2)
                        .child(line)
                })),
        )
}

/// The user's notes on their task, or the brief the assistant's worker works from: read as
/// text, Edit opens the field, saved when it is left (or the page is).
pub(super) fn notes(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let (p, task) = (part.p, part.task);
    let brief = lookup::is_assistants(task);
    let written = view.notes_from.clone();
    let editing = view.writing || written.is_empty();
    let mut head = div().flex().items_center().child(
        div()
            .flex_1()
            .child(section(p, if brief { "Brief" } else { "Description" })),
    );
    if !written.is_empty() && !view.writing {
        head = head.child(div().pt(px(10.)).child(
            link_button(p, "notes-edit".into(), "Edit").on_click(cx.listener(
                |this, _, window, cx| {
                    this.writing = true;
                    this.set_open(fold::BRIEF);
                    let focus = this.notes.read(cx).focus_handle(cx);
                    window.focus(&focus, cx);
                    cx.notify();
                },
            )),
        ));
    }
    body = body.child(head);
    if brief && !view.is_open(fold::BRIEF) && !editing {
        return body.child(
            link_button(
                p,
                "show-brief".into(),
                format!("Show the brief {APP_NAME}'s worker works from"),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.set_open(fold::BRIEF);
                cx.notify();
            })),
        );
    }
    if editing {
        return body.child(Textarea::new(&view.notes));
    }
    let title = format!(
        "{} {}",
        task.key,
        if brief { "brief" } else { "description" }
    );
    body.child(collapsible(
        p,
        part.store,
        Long {
            key: "notes",
            text: &written,
            lines: 8,
            title: Some(&title),
        },
        view,
        cx,
    ))
}

/// `History · n steps`, folded; open, newest first, each with who, the text and the time.
pub(super) fn history(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let Some(page) = part.page else {
        return body;
    };
    let (p, shown) = (part.p, history::shown(&page.history));
    body = body.child(
        fold_row(
            p,
            "history",
            format!("History · {} steps", shown.len()),
            view.is_open(fold::HISTORY),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_open(fold::HISTORY);
            cx.notify();
        })),
    );
    if !view.is_open(fold::HISTORY) {
        return body;
    }
    for (i, moment) in shown.into_iter().enumerate().rev() {
        let key = format!("history-{i}");
        body = body.child(
            div()
                .flex()
                .gap(px(10.))
                .pb(px(8.))
                .child(who(p, history::is_mine(moment)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(collapsible(
                            p,
                            part.store,
                            Long {
                                key: &key,
                                text: &moment.text,
                                lines: 4,
                                title: None,
                            },
                            view,
                            cx,
                        ))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(p.faint)
                                .child(step_time(&moment.at, part.now)),
                        ),
                ),
        );
    }
    body
}

/// `Stop Emberhut's worker` on a working task, asked first: its current run ends where it is.
pub(super) fn stop(body: Div, part: &Part<'_>, view: &TaskView, cx: &Context<'_, TaskView>) -> Div {
    let (p, task) = (part.p, part.task);
    if task.status != TaskStatus::Working {
        return body;
    }
    if !view.stopping {
        return body.child(
            div().pt(px(16.)).child(
                Button::new("stop-worker")
                    .label(format!("Stop {APP_NAME}'s worker"))
                    .small()
                    .danger()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.stopping = true;
                        cx.notify();
                    })),
            ),
        );
    }
    let id = task.id;
    body.child(
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mt(px(16.))
            .p(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.danger.opacity(0.4))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Stop {APP_NAME}'s worker on {}?", task.key)),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child("Its current run ends where it is."),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        Button::new("stop-worker-yes")
                            .label("Stop worker")
                            .small()
                            .danger()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.stopping = false;
                                this.store.update(cx, |store, cx| store.stop_worker(id, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("stop-worker-no")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.stopping = false;
                                cx.notify();
                            })),
                    ),
            ),
    )
}
