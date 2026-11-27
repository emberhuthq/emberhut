//! An external thread still running, read only: its title, status and where, its tasks (each opens its
//! page), what it was asked, its latest answer and its newest 25 steps (asked for every 10 s);
//! it finishes in the coding-thread app. A thread gone from the list and from the coding-thread app is not running anymore.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, div, px};

use super::marks::{live_dot, spinner};
use super::page::SessionPage;
use super::{Open, push};
use crate::prose::direction;
use crate::store::sessions::t3::{SHOWN, T3Watch};
use crate::store::sessions::words::short_title;
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::tab_stop;
use crate::ui::palette::{Pal, pal};
use crate::ui::reads::{self, reads};
use crate::ui::tasks::marks::glyph;

/// The page.
pub(crate) fn page(
    page: &SessionPage,
    watch: &Entity<T3Watch>,
    cx: &Context<'_, SessionPage>,
) -> gpui_kit::Stateful<Div> {
    let p = pal(cx);
    let read = watch.read(cx);
    let store = page.store.read(cx);
    let row = store
        .state()
        .and_then(|state| state.t3_running.iter().find(|t| t.id == read.thread()));
    let activity = read.activity();
    let title = row
        .map(|r| r.title.clone())
        .or_else(|| activity.map(|a| short_title(&a.title).to_owned()));
    let status = row
        .map(|r| r.status.clone())
        .or_else(|| activity.map(|a| a.status.clone()))
        .unwrap_or_default();
    let place = if read.machine().as_str() == emberhut_api::runner::t3::T3_MAC {
        "Coding app on the Mac"
    } else {
        "Coding app on the Linux PC"
    };
    let gone = row.is_none() && read.has_asked() && activity.is_none();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(16.))
        .child(
            reads(&title.unwrap_or_else(|| "External thread".to_owned()))
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(live_dot(&p, &status, "t3-dot", cx))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if gone {
                            "Not running anymore".to_owned()
                        } else {
                            crate::format::capitalized(&status)
                        }),
                )
                .child(div().flex_1())
                .child(div().text_size(px(12.)).text_color(p.muted).child(place)),
        );
    for task in row.map_or_default(|r| r.tasks.clone()) {
        body = body.child(task_link(page, task, &p, cx));
    }
    body = body.child(
        div()
            .text_size(px(12.))
            .text_color(p.faint)
            .child(format!(
                "Read only: this thread finishes in the coding-thread app. New work runs on {}'s runner, where you can watch it live.",
                crate::brand::APP_NAME
            )),
    );
    if let Some(ask) = row.and_then(|r| r.ask.clone()) {
        body = body.child(section(&p, "Asked", ask));
    }
    if let Some(last) = row.and_then(|r| r.last.clone()) {
        body = body.child(
            div()
                .flex()
                .flex_col()
                .when(reads::starts_right(&last), Styled::text_right)
                .gap(px(4.))
                .child(heading(&p, "Latest answer"))
                .child(crate::ui::prose::markdown_view(
                    "t3-last".into(),
                    &last,
                    &page.store,
                )),
        );
    }
    match activity {
        Some(activity) if !activity.items.is_empty() => body = body.child(steps(&p, activity)),
        None if !read.has_asked() => body = body.child(spinner(&p, "t3-loading", 16., cx)),
        Some(_) | None => {}
    }
    div()
        .id("t3-page")
        .size_full()
        .overflow_y_scroll()
        .child(body)
}

/// A thread of work in flight not on a runner, read only: its state, what it was asked and its
/// latest answer; it lives in the coding-thread app on its machine. A thread gone from the state is not
/// running anymore.
pub(crate) fn work(
    page: &SessionPage,
    id: &emberhut_api::ids::WorkId,
    cx: &Context<'_, SessionPage>,
) -> gpui_kit::Stateful<Div> {
    let p = pal(cx);
    let store = page.store.read(cx);
    let row = store
        .state()
        .and_then(|state| state.working.iter().find(|row| &row.id == id));
    let mut body = div().flex().flex_col().gap(px(12.)).p(px(16.));
    match row {
        None => {
            body = body
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child("Not running anymore"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child("This thread finished or stopped."),
                );
        }
        Some(row) => {
            let place = if row
                .machine
                .as_ref()
                .map(emberhut_api::ids::MachineId::as_str)
                == Some(emberhut_api::runner::t3::T3_MAC)
            {
                "the Mac"
            } else {
                "the Linux PC"
            };
            body = body
                .child(
                    reads(&row.title)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.)),
                )
                .when_some(row.line.clone(), |d, line| {
                    d.child(div().text_size(px(12.5)).text_color(p.muted).child(line))
                });
            if let Some(ask) = row.ask.clone() {
                body = body.child(section(&p, "Asked", ask));
            }
            if let Some(last) = row.last.clone() {
                body = body.child(section(&p, "Latest answer", last));
            }
            body = body.child(div().text_size(px(12.)).text_color(p.faint).child(format!(
                "Open the coding-thread app on {place} to read or answer the whole thread."
            )));
        }
    }
    div()
        .id("work-page")
        .size_full()
        .overflow_y_scroll()
        .child(body)
}

/// One of its tasks: a click opens its page.
fn task_link(
    page: &SessionPage,
    task: emberhut_api::ids::TaskId,
    p: &Pal,
    cx: &gpui_kit::App,
) -> impl IntoElement {
    let store = page.store.read(cx);
    let label = store.task_of(task).map_or_else(
        || store.task_key(task),
        |issue| format!("{} {}", issue.key, issue.title),
    );
    let owner = page.store.clone();
    tab_stop(
        div()
            .id(ElementId::Name(format!("t3-task-{task}").into()))
            .button_named(format!("Task {label}"))
            .hint("Opens its page")
            .border_1()
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .text_color(p.accent)
            .child(div().text_size(px(12.)).text_color(p.muted).child("Task"))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(13.))
                    .child(direction::marked(&label)),
            )
            .child(glyph(IconName::ChevronRight, p.accent, 12.))
            .on_click(move |_, window, cx| push(&owner, Open::Task(task), window, cx)),
        p,
    )
}

/// The thread's newest steps, newest first.
fn steps(p: &Pal, activity: &emberhut_api::threads::ThreadActivity) -> Div {
    let mut steps = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(heading(p, "Latest steps"));
    for item in activity.items.iter().rev().take(SHOWN) {
        steps = steps.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted)
                        .child(item.kind_name()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_clamp(6)
                        .when(item.is_command(), |d| d.font_family("monospace"))
                        .when(
                            !item.is_command() && reads::starts_right(&item.text),
                            Styled::text_right,
                        )
                        .child(item.text.clone()),
                ),
        );
    }
    steps
}

fn heading(p: &Pal, text: &'static str) -> Div {
    div()
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.muted)
        .child(text)
}

fn section(p: &Pal, title: &'static str, text: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(heading(p, title))
        .child(
            div()
                .text_size(px(13.))
                .line_clamp(4)
                .when(reads::starts_right(&text), Styled::text_right)
                .child(text),
        )
}
