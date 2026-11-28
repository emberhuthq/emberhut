//! The top of a task's page on a server that keeps plans, so the first screen says what is
//! done, what runs now, what is next and what needs the user:
//!
//! - the head under the title, a few short lines: its state (amber spinner while it runs, and
//!   `quiet 25m` when nothing moved for a while), Now (the current step), Needs you or
//!   Blocked, the ETA, and the session at work with Open session;
//! - the plan: its summary (`4 of 9 done · 1 blocked · 1 skipped`) over a bar of one segment
//!   a step, then the checklist: done steps ticked and dimmed, the current one lit amber with
//!   how long it has run, a blocked one with what blocks it, a skipped one struck through,
//!   sub-steps indented, a step's result as a chip. A long plan folds around the current step
//!   (`5 done ›`, the last done one, the current one, the next two, `3 more ›`);
//! - an empty plan's "No plan yet", and quietly when a turn of work ended without one;
//! - the decision in a card of its own when the task waits on the user.

use emberhut_api::task_plan::{PlanStep, StepState};
use emberhut_api::tasks::TaskStatus;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, FontWeight, Hsla, div, px};

use super::text::{follow, rich};
use super::{Part, TaskView, fold};
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::sessions::words::short_title;
use crate::store::tasks::plan::{self, Item, StepRow};
use crate::store::tasks::{clock, words};
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::reads::reads;
use crate::ui::sessions::{self, Open};
use crate::ui::tasks::marks::{glyph, status_icon, tone_color};
use crate::ui::tasks::plan_marks;

/// The title and the head's lines.
pub(super) fn head(body: Div, part: &Part<'_>, view: &TaskView) -> Div {
    let (p, task) = (part.p, part.task);
    let now = part.now.timestamp();
    let full = part.page.and_then(|page| page.plan.as_ref());
    let mut lines: Vec<(&'static str, Div)> = vec![("State", state_line(part, view.turn))];
    let current = full.and_then(plan::now_line).or_else(|| {
        task.plan
            .as_ref()
            .and_then(|summary| summary.current.as_ref())
            .map(|step| step.title.clone())
    });
    if let Some(current) = current {
        lines.push(("Now", words_of(current, p.text, true)));
    }
    if task.status == TaskStatus::Waiting {
        if let Some(action) = &task.action {
            // The action's keys and links open as they do anywhere on the page.
            lines.push((
                "Needs you",
                div()
                    .text_color(p.accent)
                    .font_weight(FontWeight::MEDIUM)
                    .child(rich("head-action", action, Some(2), part.store)),
            ));
        } else if let Some(decision) = &task.decision {
            lines.push((
                "Needs you",
                words_of(decision.question.clone(), p.accent, true),
            ));
        }
    }
    if let Some(blocker) = task
        .plan
        .as_ref()
        .and_then(|summary| summary.blocker.clone())
    {
        lines.push(("Blocked", words_of(blocker, p.danger, false)));
    }
    // Where it runs or when it looks next, what it waits on, what it was asked, the last run:
    // the lines the page's live card has without plans.
    for (label, value) in super::top::live_lines(part) {
        lines.push((label, words_of(value, p.text, false)));
    }
    let summary = task.plan.as_ref();
    let updated = summary.and_then(|summary| plan::eta_line(summary, now));
    // The plan's estimate, else the task's own; without either the line only says when the
    // plan last moved.
    let eta = summary
        .and_then(|summary| summary.eta.clone())
        .or_else(|| task.eta.clone());
    match (eta, updated) {
        (Some(_), Some(line)) if summary.is_some_and(|s| s.eta.is_some()) => {
            lines.push(("ETA", words_of(line, p.muted, false)));
        }
        (Some(eta), Some(line)) => {
            lines.push(("ETA", words_of(format!("{eta} · {line}"), p.muted, false)));
        }
        (Some(eta), None) => lines.push(("ETA", words_of(eta, p.muted, false))),
        (None, Some(line)) => lines.push((
            "Updated",
            words_of(
                line.trim_start_matches("updated ").to_owned(),
                p.muted,
                false,
            ),
        )),
        (None, None) => {}
    }
    let mut head = div().flex().flex_col().gap(px(5.)).pt(px(10.));
    for (label, value) in lines {
        head = head.child(
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .text_size(px(12.5))
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .text_color(p.muted)
                        .child(label),
                )
                .child(value.flex_1().min_w_0()),
        );
    }
    body.child(
        reads(&task.title)
            .pt(px(14.))
            .text_size(px(18.))
            .font_weight(FontWeight::BOLD),
    )
    .child(head)
    .children(session_line(part))
}

/// A head line's words.
fn words_of(words: String, color: Hsla, strong: bool) -> Div {
    div()
        .line_clamp(2)
        .text_color(color)
        .when(strong, |d| d.font_weight(FontWeight::MEDIUM))
        .child(words)
}

/// The state line: its mark, its name and for how long, and `quiet 25m` when it has gone quiet.
fn state_line(part: &Part<'_>, turn: u8) -> Div {
    let (p, task) = (part.p, part.task);
    let now = part.now.timestamp();
    let running = words::doing(task) == words::Doing::Running;
    let mark = if running {
        plan_marks::running(p, 12., turn).into_any_element()
    } else {
        status_icon(p, task).into_any_element()
    };
    let since = task
        .live
        .as_ref()
        .and_then(|live| live.since.as_ref())
        .and_then(clock::instant)
        .map(|since| clock::span(since, now));
    let name = [Some(words::name(task)), since]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let quiet = part.read.state().and_then(|state| {
        let sessions = crate::store::sessions::lists::all(state);
        plan::quiet(task, &sessions, now)
    });
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(mark)
        .child(
            div()
                .text_color(tone_color(p, words::tone(task)))
                .font_weight(FontWeight::MEDIUM)
                .child(name),
        )
        .children(quiet.map(|quiet| {
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .text_color(p.muted)
                .child(glyph(IconName::Clock, p.muted, 11.))
                .child(quiet)
        }))
}

/// The session at work: `<title> on <machine> · updated 1 minute ago · Open session ›`; a
/// running one first, else the newest.
fn session_line(part: &Part<'_>) -> Option<gpui_kit::Stateful<Div>> {
    let p = part.p;
    let sessions = &part.page?.sessions;
    let session = sessions
        .iter()
        .find(|session| session.is_running())
        .or_else(|| {
            sessions
                .iter()
                .max_by(|a, b| a.updated_at.as_str().cmp(b.updated_at.as_str()))
        })?;
    let mut words = short_title(&session.title).to_owned();
    if !session.place.is_empty() {
        words = format!("{words} on {}", session.place);
    }
    if let Some(at) = clock::instant(&session.updated_at) {
        words = format!(
            "{words} · updated {}",
            clock::relative(at, part.now.timestamp())
        );
    }
    let place = SessionPlace::Session(SessionRef::whole(session.id.clone()));
    let store = part.store.clone();
    Some(tab_stop(
        div()
            .id("task-head-session")
            .button_named(format!("Open session: {words}"))
            .hint("Opens the session at work on this task")
            .border_1()
            .rounded(px(7.))
            .mt(px(6.))
            .px(px(6.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(glyph(IconName::SquareTerminal, p.faint, 13.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(p.muted)
                    .child(words),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(p.accent)
                    .child("Open session ›"),
            )
            .on_click(move |_, window, cx| {
                sessions::push(&store, Open::Place(place.clone()), window, cx);
            }),
        p,
    ))
}

/// The plan: its summary, its bar and its checklist; "No plan yet" while it is empty. Nothing
/// on a server that keeps no plans.
pub(super) fn section_of(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let full = part.page.and_then(|page| page.plan.as_ref());
    let Some(summary) = full.map(|plan| &plan.summary).or(part.task.plan.as_ref()) else {
        return body;
    };
    let p = part.p;
    body = body.child(super::section(p, "Plan").pt(px(18.)));
    // A plan is empty when it has no step; one whose every step is skipped has a total of 0
    // but still shows. Until the page comes, the summary says.
    if full.map_or(summary.is_empty(), |full| full.steps.is_empty()) {
        return body.child(empty(part));
    }
    let (line, bar) = match full {
        Some(full) => (plan::summary_line(full), plan::bar_of_plan(full)),
        None => (
            format!("{} of {} done", summary.done, summary.total),
            plan::bar_of_summary(summary, false),
        ),
    };
    let (count, rest) = line
        .split_once(" · ")
        .map_or((line.as_str(), None), |(count, rest)| (count, Some(rest)));
    body = body.child(
        div()
            .flex()
            .items_baseline()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(count.to_owned()),
            )
            .children(rest.map(|rest| {
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child(rest.to_owned())
            })),
    );
    if let Some(bar) = bar {
        body = body.child(
            div()
                .pt(px(6.))
                .pb(px(8.))
                .child(plan_marks::bar(p, &bar, 5.)),
        );
    }
    let Some(full) = full else {
        return body;
    };
    let items = plan::checklist(
        full,
        view.is_open(fold::PLAN_EARLIER),
        view.is_open(fold::PLAN_LATER),
    );
    let mut list = div()
        .id("plan-steps")
        .read_as(gpui_kit::Role::List, "Plan")
        .flex()
        .flex_col()
        .gap(px(1.));
    for item in items {
        list = list.child(match item {
            Item::Step(row) => step_row(part, &row).into_any_element(),
            Item::Earlier { count, all_done } => {
                let words = if all_done {
                    format!("{count} done ›")
                } else {
                    format!("{count} earlier ›")
                };
                unfold(p, "plan-earlier", words, fold::PLAN_EARLIER, cx).into_any_element()
            }
            Item::Later(count) => unfold(
                p,
                "plan-later",
                format!("{count} more ›"),
                fold::PLAN_LATER,
                cx,
            )
            .into_any_element(),
        });
    }
    body.child(list)
}

/// An empty plan: a dashed box with "No plan yet" and, on the assistant's task, that it writes
/// the steps when it starts (the user's own to-do says `To do · No plan yet`), and when a turn
/// ended without one.
fn empty(part: &Part<'_>) -> Div {
    let p = part.p;
    let mine = !crate::store::lookup::is_assistants(part.task);
    let unplanned = part
        .task
        .plan
        .as_ref()
        .and_then(|summary| plan::unplanned_line(summary, part.now.timestamp()));
    let ghost = || {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .size(px(14.))
                    .rounded_full()
                    .border_1()
                    .border_color(p.border),
            )
            .child(div().h(px(6.)).w(px(140.)).rounded_full().bg(p.border))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(14.))
        .rounded(px(12.))
        .border_1()
        .border_dashed()
        .border_color(p.border)
        .child(ghost())
        .child(ghost())
        .child(
            div()
                .pt(px(4.))
                .text_size(px(13.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if mine {
                    format!("To do · {}", plan::NO_PLAN)
                } else {
                    plan::NO_PLAN.to_owned()
                }),
        )
        // The assistant writes the steps of its own tasks only.
        .when(!mine, |d| {
            d.child(div().text_size(px(12.5)).text_color(p.muted).child(format!(
                "{} writes the steps when it starts.",
                crate::brand::APP_NAME
            )))
        })
        .children(unplanned.map(|line| div().text_size(px(11.5)).text_color(p.faint).child(line)))
}

/// A folded part of the checklist: a click opens it.
fn unfold(
    p: &Pal,
    id: &'static str,
    words: String,
    key: &'static str,
    cx: &Context<'_, TaskView>,
) -> gpui_kit::Stateful<Div> {
    tab_stop(
        div()
            .id(id)
            .button_named(words.trim_end_matches(" ›").to_owned())
            .aria_expanded(false)
            .border_1()
            .rounded(px(7.))
            .flex()
            .items_center()
            .gap(px(11.))
            .py(px(5.))
            .px(px(8.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .text_size(px(12.5))
            .text_color(p.muted)
            .child(div().w(px(18.)).flex().justify_center().child(glyph(
                IconName::ChevronsUpDown,
                p.faint,
                13.,
            )))
            .child(words)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_open(key);
                cx.notify();
            })),
        p,
    )
}

/// A step's tick: a filled disc with a check when done, an amber ring around an amber core
/// while being worked on, a red ring with `!` when blocked, a faint dashed ring with a dash when
/// skipped, a ring to do.
fn tick(p: &Pal, state: &StepState, size: f32) -> Div {
    let ring = |color: Hsla| {
        div()
            .size(px(size))
            .flex_none()
            .rounded_full()
            .border_color(color)
            .flex()
            .items_center()
            .justify_center()
    };
    match state {
        StepState::Done => {
            ring(p.success)
                .bg(p.success)
                .child(glyph(IconName::Check, p.surface, size * 0.62))
        }
        StepState::Doing => ring(p.warn)
            .border_2()
            .child(div().size(px(size * 0.42)).rounded_full().bg(p.warn)),
        StepState::Blocked => ring(p.danger)
            .border_2()
            .text_size(px(size * 0.6))
            .font_weight(FontWeight::EXTRA_BOLD)
            .text_color(p.danger)
            .child("!"),
        StepState::Skipped => ring(p.faint).border_1().border_dashed().child(glyph(
            IconName::Minus,
            p.faint,
            size * 0.6,
        )),
        _ => ring(p.muted).border_1(),
    }
}

/// One step of the checklist.
fn step_row(part: &Part<'_>, row: &StepRow<'_>) -> gpui_kit::Stateful<Div> {
    let (p, step) = (part.p, row.step);
    let size = if row.nested { 15. } else { 18. };
    let lit = row.current && step.state == StepState::Doing;
    let title_color = match step.state {
        StepState::Done => p.muted,
        StepState::Skipped => p.faint,
        _ => p.text,
    };
    let so_far = lit
        .then(|| plan::so_far(step, part.now.timestamp()))
        .flatten();
    let title = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(if row.nested { 13. } else { 13.5 }))
                .text_color(title_color)
                .when(lit, |d| d.font_weight(FontWeight::SEMIBOLD))
                .when(step.state == StepState::Skipped, Styled::line_through)
                .child(step.title.clone()),
        )
        .children(step.link.clone().map(|link| result_chip(part, step, link)))
        .children(so_far.map(|so_far| {
            div()
                .flex_none()
                .text_size(px(11.5))
                .text_color(p.warn)
                .child(so_far)
        }));
    div()
        .id(ElementId::Name(
            plan::step_key(part.task.id, step.id).into(),
        ))
        .read_as(gpui_kit::Role::ListItem, plan::spoken(step))
        .flex()
        .items_start()
        .gap(px(11.))
        .py(px(6.))
        .px(px(8.))
        .when(row.nested, |d| d.ml(px(29.)))
        .rounded(px(8.))
        .when(lit, |d| d.bg(p.warn.opacity(0.09)))
        .when(step.state == StepState::Blocked, |d| {
            d.bg(p.danger.opacity(0.06))
        })
        .child(
            div()
                .h(px(19.))
                .flex()
                .items_center()
                .child(tick(p, &step.state, size)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .min_w_0()
                .child(title)
                .children(below(part, step)),
        )
}

/// What a step says under its title: `Now: <note>` while it is worked on, what blocks it (and
/// that the answer is below when the task asks), `Skipped: <note>`, else its note.
fn below(part: &Part<'_>, step: &PlanStep) -> Option<Div> {
    let p = part.p;
    let note = step
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty());
    let line = div().text_size(px(12.)).line_clamp(3);
    Some(match step.state {
        StepState::Doing => line.text_color(p.muted).child(format!("Now: {}", note?)),
        StepState::Blocked => {
            let asks = part.task.status == TaskStatus::Waiting && part.task.decision.is_some();
            line.flex()
                .items_start()
                .gap(px(4.))
                .text_color(p.danger)
                .child(glyph(IconName::TriangleAlert, p.danger, 12.))
                .child(div().child(note.unwrap_or("Blocked").to_owned()))
                .when(asks, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_color(p.accent)
                            .font_weight(FontWeight::MEDIUM)
                            .child("Answer below ↓"),
                    )
                })
        }
        StepState::Skipped => line
            .text_color(p.faint)
            .child(format!("Skipped: {}", note?)),
        _ => line.text_color(p.muted).child(note?.to_owned()),
    })
}

/// A step's result as a chip: its link in short words, opened as any link on the page is.
fn result_chip(part: &Part<'_>, step: &PlanStep, link: String) -> gpui_kit::Stateful<Div> {
    let p = part.p;
    let store = part.store.clone();
    let label = plan::link_label(&link);
    tab_stop(
        div()
            .id(ElementId::Name(
                format!("{}-link", plan::step_key(part.task.id, step.id)).into(),
            ))
            .button_named(format!("Open {label}"))
            .hint("Opens what this step produced")
            .flex_none()
            .max_w(px(220.))
            .border_1()
            .border_color(p.border)
            .rounded(px(6.))
            .px(px(6.))
            .py(px(1.))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(11.5))
            .text_color(p.muted)
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(glyph(IconName::Link, p.faint, 11.))
            .child(div().truncate().child(label))
            .on_click(move |_, window, cx| follow(&store, &link, window, cx)),
        p,
    )
}

/// The decision, when the task waits on the user, in a card of its own under the plan:
/// `Needs you` over what it asks.
pub(super) fn needs_you(
    body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let task = part.task;
    if task.status != TaskStatus::Waiting || task.decision.is_none() {
        return super::top::asks(body, part, view, cx);
    }
    let p = part.p;
    let card = div()
        .mt(px(16.))
        .p(px(14.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.accent.opacity(0.35))
        .bg(p.accent_soft.opacity(0.55))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.accent)
                .child(glyph(IconName::Hand, p.accent, 13.))
                .child("Needs you"),
        );
    body.child(super::top::asks(card, part, view, cx))
}
