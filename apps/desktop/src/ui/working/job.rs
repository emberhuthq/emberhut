//! A background job's page: its task (a click opens it), how it runs now (for how long, since
//! when, CPU, memory, the ports it serves), what it last said and changed, where it runs (its
//! name, folder and command, selectable), and Stop or Clear, each asked first. A job that
//! ended stays on its page, marked Ended.

use emberhut_api::jobs::{Job, JobAction, JobActivity};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, FontWeight, Role, ScrollHandle, div, px};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::jobs::{job_color, job_icon};
use crate::prose::direction;
use crate::store::devices::relative;
use crate::store::working::jobs::{
    JobKey, Shown, confirm_words, cpu, memory, ran, short_span, state_word,
};
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::reads::reads;
use crate::ui::settings::parts::selectable;
use crate::ui::widgets::scroller;

/// The page of `key`'s job.
pub(crate) fn page(
    store: &Entity<Store>,
    key: JobKey,
    scroll: &ScrollHandle,
    p: &Pal,
    cx: &App,
) -> Div {
    let read = store.read(cx);
    let feed = read.jobs();
    let body = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(720.));
    let body = match feed.shown(key) {
        Some(Shown::Listed(job)) => body.children(sections(store, read, job, false, p)),
        Some(Shown::Gone(job)) => body.children(sections(store, read, job, true, p)),
        None if feed.page().is_none() => body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_color(p.muted)
                .child(Spinner::new().small().color(p.muted))
                .child("Looking…"),
        ),
        None => body.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .py(px(40.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Not running anymore"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child("This job ended."),
                ),
        ),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(scroller("job-scroll", scroll).child(body))
}

/// A section: its title over its rows.
fn titled(title: &'static str, rows: Vec<AnyElement>, p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(12.))
                .rounded(px(10.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .children(rows),
        )
}

/// A label and its value on one line.
fn fact(label: &str, value: String, p: &Pal) -> AnyElement {
    div()
        .flex()
        .gap(px(12.))
        .text_size(px(13.))
        .child(div().flex_1().child(label.to_owned()))
        .child(div().text_color(p.muted).child(value))
        .into_any_element()
}

/// The page's sections; `gone`: it ended and isn't on the list anymore.
fn sections(
    store: &Entity<Store>,
    read: &Store,
    job: &Job,
    gone: bool,
    p: &Pal,
) -> Vec<AnyElement> {
    let now = Timestamp::now();
    let mut parts = vec![head(job, gone, p).into_any_element()];
    parts.push(task(read, job, p).into_any_element());
    let mut running = Vec::new();
    if let Some(since) = job
        .since
        .as_ref()
        .and_then(|at| at.as_str().parse::<Timestamp>().ok())
    {
        let label = if job.is_live() {
            "Running for"
        } else {
            "Ran for"
        };
        running.push(fact(label, ran(job, now).unwrap_or_default(), p));
        let started = since
            .to_zoned_datetime(TimeZone::system())
            .strftime("%b %-d, %Y, %-I:%M %p")
            .to_string();
        running.push(fact("Started", started, p));
    }
    if let Some(cpu) = job.cpu {
        running.push(fact(
            "CPU now",
            format!("{}%", crate::format::whole(cpu)),
            p,
        ));
    }
    if let Some(time) = job.cpu_time {
        running.push(fact("CPU time", short_span(time), p));
    }
    if let Some(memory) = memory(job) {
        running.push(fact("Memory", memory, p));
    }
    if !job.ports.is_empty() {
        let served: Vec<String> = job.ports.iter().map(u16::to_string).collect();
        running.push(fact("Serving on", served.join(", "), p));
    }
    if running.is_empty()
        && let Some(cpu) = cpu(job)
    {
        running.push(fact("CPU", cpu, p));
    }
    if !running.is_empty() {
        parts.push(titled("Now", running, p).into_any_element());
    }
    let mut activity = Vec::new();
    if let Some(last) = &job.last {
        let label = if last.is_log() { "Its log" } else { "It said" };
        activity.push(said(job, "last", label, last, now, p));
    }
    if let Some(file) = &job.file {
        activity.push(said(job, "file", "Last file change", file, now, p));
    }
    if !activity.is_empty() {
        parts.push(titled("Last activity", activity, p).into_any_element());
    }
    parts.push(place(job, p).into_any_element());
    if !gone && (job.stop || job.clear) {
        parts.push(actions(store, read, job, p).into_any_element());
    }
    parts
}

/// Its icon, title, state, what it is and where, how it ended.
fn head(job: &Job, gone: bool, p: &Pal) -> Div {
    let state = if gone {
        "Ended".to_owned()
    } else {
        state_word(&job.state)
    };
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            Icon::new(job_icon(job))
                .size(px(24.))
                .text_color(job_color(job, p)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .flex_1()
                .min_w_0()
                .child(
                    reads(&job.title)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.)),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(if job.is_failed() { p.danger } else { p.muted })
                        .child(format!("{state} · {} · {}", job.what, job.place)),
                )
                .when_some(job.exit.clone(), |d, exit| {
                    d.child(div().text_size(px(12.)).text_color(p.danger).child(exit))
                }),
        )
}

/// Its task, opening on a click; or how a job gets one.
fn task(read: &Store, job: &Job, p: &Pal) -> Div {
    let row: AnyElement = match &job.task {
        Some(task) => {
            let id = task.id;
            let name = format!("{} · {}", read.task_key(id), task.title);
            let status = crate::format::capitalized(&task.status);
            tab_stop(
                div()
                    .id("job-task")
                    .button_named(a11y::line(&[&name, &status]))
                    .hint("Opens its page")
                    .border_1()
                    .rounded(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(p.accent)
                            .child(direction::marked(&name)),
                    )
                    .child(div().text_size(px(11.5)).text_color(p.muted).child(status))
                    .on_click(move |_, window, cx| column::push(Page::Task(id), window, cx)),
                p,
            )
            .into_any_element()
        }
        None => div()
            .text_size(px(12.))
            .text_color(p.muted)
            .child(
                "Not tied to a task. A job named after a task, or with its key in its \
                 description, shows its task.",
            )
            .into_any_element(),
    };
    titled("Task", vec![row], p)
}

/// What it said or changed last, and when.
fn said(
    job: &Job,
    which: &str,
    label: &'static str,
    activity: &JobActivity,
    now: Timestamp,
    p: &Pal,
) -> AnyElement {
    let at = activity
        .at
        .as_ref()
        .and_then(|at| at.as_str().parse::<Timestamp>().ok())
        .map(|at| relative(at, now));
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .text_size(px(11.5))
                .child(div().flex_1().text_color(p.muted).child(label))
                .children(at.map(|at| div().text_color(p.faint).child(at))),
        )
        .child(div().text_size(px(12.5)).child(selectable(
            ElementId::Name(format!("job-{}-{which}", job.id).into()),
            &activity.text,
        )))
        .into_any_element()
}

/// Where it runs: its name, folder and command, selectable.
fn place(job: &Job, p: &Pal) -> Div {
    let mut rows = vec![
        div()
            .flex()
            .gap(px(12.))
            .text_size(px(13.))
            .child(div().flex_1().child("Name"))
            .child(div().text_color(p.muted).child(selectable(
                ElementId::Name(format!("job-{}-name", job.id).into()),
                &job.name,
            )))
            .into_any_element(),
    ];
    if let Some(cwd) = &job.cwd {
        rows.push(
            div()
                .flex()
                .gap(px(12.))
                .text_size(px(13.))
                .child(div().flex_1().child("Folder"))
                .child(div().text_color(p.muted).child(selectable(
                    ElementId::Name(format!("job-{}-cwd", job.id).into()),
                    cwd,
                )))
                .into_any_element(),
        );
    }
    if let Some(command) = &job.command {
        rows.push(
            div()
                .text_size(px(11.5))
                .text_color(p.muted)
                .line_clamp(6)
                .child(selectable(
                    ElementId::Name(format!("job-{}-command", job.id).into()),
                    command,
                ))
                .into_any_element(),
        );
    }
    titled("Where", rows, p)
}

/// Stop and Clear, the question before either, and why one wasn't done.
fn actions(store: &Entity<Store>, read: &Store, job: &Job, p: &Pal) -> Div {
    let feed = read.jobs();
    let busy = feed.busy().is_some();
    let mut rows: Vec<AnyElement> = Vec::new();
    let ask = |action: JobAction| {
        let (store, id) = (store.clone(), job.id.clone());
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut App| {
            let id = id.clone();
            store.update(cx, |store, cx| store.confirm_job(Some((id, action)), cx));
        }
    };
    let mut buttons = div().flex().items_center().gap(px(10.));
    if job.stop {
        buttons = buttons.child(
            Button::new("job-stop")
                .small()
                .danger()
                .label("Stop")
                .disabled(busy)
                .on_click(ask(JobAction::Stop)),
        );
    }
    if job.clear {
        buttons = buttons.child(
            Button::new("job-clear")
                .small()
                .label("Clear")
                .disabled(busy)
                .on_click(ask(JobAction::Clear)),
        );
    }
    if feed.busy() == Some(&job.id) {
        buttons = buttons.child(
            div()
                .id("job-busy")
                .announced("Working…")
                .child(Spinner::new().small().color(p.muted)),
        );
    }
    rows.push(buttons.into_any_element());
    if let Some(action) = feed.confirming(&job.id) {
        let key = job.task.as_ref().map(|task| read.task_key(task.id));
        rows.push(question(store, job, action, key.as_deref(), p).into_any_element());
    }
    let foot = match feed.refused(&job.id) {
        Some(why) => Some((why.to_owned(), p.danger)),
        None => job.clear.then(|| {
            (
                "Clear takes a failed job off the list. Nothing else changes.".to_owned(),
                p.muted,
            )
        }),
    };
    if let Some((text, color)) = foot {
        rows.push(
            div()
                .id("job-foot")
                .when(feed.refused(&job.id).is_some(), |d| {
                    d.announced(text.clone())
                })
                .text_size(px(11.5))
                .text_color(color)
                .child(text)
                .into_any_element(),
        );
    }
    div().flex().flex_col().gap(px(8.)).children(rows)
}

/// The question before a Stop or Clear: what happens, then the action and Cancel.
fn question(
    store: &Entity<Store>,
    job: &Job,
    action: JobAction,
    key: Option<&str>,
    p: &Pal,
) -> gpui_kit::Stateful<Div> {
    let (title, words) = confirm_words(job, action, key);
    let (yes, no, id) = (store.clone(), store.clone(), job.id.clone());
    let button = match action {
        JobAction::Stop => Button::new("job-confirm").small().danger().label("Stop it"),
        JobAction::Clear => Button::new("job-confirm").small().primary().label("Clear"),
    };
    div()
        .id("job-question")
        .read_as(Role::AlertDialog, title.clone())
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(p.danger.opacity(0.08))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_size(px(12.)).text_color(p.muted).child(words))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(button.on_click(move |_, _, cx| {
                    let id = id.clone();
                    yes.update(cx, |store, cx| store.act_on_job(id, action, cx));
                }))
                .child(
                    Button::new("job-cancel")
                        .small()
                        .ghost()
                        .label("Cancel")
                        .on_click(move |_, _, cx| {
                            no.update(cx, |store, cx| store.confirm_job(None, cx));
                        }),
                ),
        )
}
