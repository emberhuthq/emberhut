//! Background jobs: every job running on the machines now, live while it shows: the work
//! first (failed ones on top, in red, counted), each with its task, its last activity, how
//! long it has run with its CPU and memory and where; then the services and file servers,
//! folded. A row opens the job's page. The Mac's trouble, or why the list didn't come, at the
//! foot.

use std::time::Duration;

use emberhut_api::jobs::{Job, JobKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Div, ElementId, Entity, FontWeight, Hsla,
    ScrollHandle, div, pulsating_between, px,
};
use jiff::Timestamp;

use crate::prose::direction;
use crate::store::devices::relative;
use crate::store::working::jobs::{facts, last_active, split};
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::widgets::{page_head, pill, scroller};

/// A job's icon.
#[must_use]
pub(crate) fn job_icon(job: &Job) -> IconName {
    if job.is_failed() {
        return IconName::CircleX;
    }
    match job.kind {
        JobKind::Worker => IconName::CircleUser,
        JobKind::Session => IconName::SquareTerminal,
        JobKind::Server => IconName::Network,
        JobKind::Service => IconName::Settings2,
        JobKind::Launchd => IconName::Frame,
        _ => IconName::Settings,
    }
}

/// A job's colour: failed red, running teal, else muted.
#[must_use]
pub(crate) fn job_color(job: &Job, p: &Pal) -> Hsla {
    if job.is_failed() {
        p.danger
    } else if job.is_live() {
        crate::ui::palette::category_color(5)
    } else {
        p.muted
    }
}

/// The page.
pub(crate) fn page(store: &Entity<Store>, scroll: &ScrollHandle, p: &Pal, cx: &App) -> Div {
    let read = store.read(cx);
    let feed = read.jobs();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(760.));
    match feed.page() {
        None => {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(p.muted)
                    .when(feed.problem().is_none(), |d| {
                        d.child(Spinner::new().small().color(p.muted))
                    })
                    .child(
                        feed.problem()
                            .unwrap_or("Looking at what runs on your machines…")
                            .to_owned(),
                    ),
            );
        }
        Some(page) => {
            let (work, services) = split(&page.jobs);
            let running = work.iter().filter(|job| job.is_live()).count();
            let failed = work.iter().filter(|job| job.is_failed()).count();
            body = body.child(header(running, failed, p));
            if work.is_empty() {
                body = body.child(
                    div()
                        .text_color(p.muted)
                        .text_size(px(13.))
                        .child("No background jobs running."),
                );
            }
            let now = Timestamp::now();
            let still = crate::motion::reduced(cx);
            for job in work {
                body = body.child(row(store, read, job, (now, still), p));
            }
            if !services.is_empty() {
                body = body.child(services_fold(
                    store,
                    services.len(),
                    feed.services_open(),
                    p,
                ));
                if feed.services_open() {
                    for job in services {
                        body = body.child(row(store, read, job, (now, still), p));
                    }
                }
            }
            let foot = match (&page.mac.error, feed.problem()) {
                (Some(error), _) if !page.mac.ok => Some((format!("The Mac: {error}"), p.muted)),
                (_, Some(problem)) => Some((problem.to_owned(), p.danger)),
                _ => None,
            };
            if let Some((text, color)) = foot {
                body = body.child(
                    div()
                        .id("jobs-problem")
                        .announced(text.clone())
                        .text_size(px(12.))
                        .text_color(color)
                        .child(text),
                );
            }
        }
    }
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(page_head(
            p,
            "Background jobs",
            Some("live, every 10 s".into()),
        ))
        .child(scroller("jobs-scroll", scroll).child(body))
}

/// `Background · 2` and `1 failed`.
fn header(running: usize, failed: usize, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.muted)
        .child("Background")
        .child(
            div()
                .text_color(crate::ui::palette::category_color(5))
                .child(running.to_string()),
        )
        .when(failed > 0, |d| {
            d.child(pill(format!("{failed} failed"), p.danger, p.on_accent))
        })
}

/// The services' fold: `Services · 3`, what they are.
fn services_fold(store: &Entity<Store>, count: usize, open: bool, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    let chevron = if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    tab_stop(
        div()
            .id("jobs-services")
            .button_named(format!("Services, {count}"))
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .border_1()
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .p(px(8.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(Icon::new(chevron).size(px(12.)).text_color(p.faint))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("Services · {count}")),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.muted)
                            .child("Always on: file pages, bots, feeds"),
                    ),
            )
            .on_click(move |_, _, cx| store.update(cx, Store::toggle_services)),
        p,
    )
}

/// One job: its icon (pulsing while it works, unless the system asks things to hold `still`),
/// title, task, last activity, facts and when it was last active, as of `now`. A click opens
/// its page.
fn row(
    store: &Entity<Store>,
    read: &Store,
    job: &Job,
    (now, still): (Timestamp, bool),
    p: &Pal,
) -> impl IntoElement {
    let color = job_color(job, p);
    let glyph = Icon::new(job_icon(job)).size(px(16.)).text_color(color);
    let mark: AnyElement = if job.is_live() && !job.is_quiet() && !still {
        glyph
            .with_animation(
                ElementId::Name(format!("job-pulse-{}", job.id).into()),
                Animation::new(Duration::from_millis(1600))
                    .repeat()
                    .with_easing(pulsating_between(0.4, 1.)),
                move |icon, delta| icon.text_color(color.opacity(delta)),
            )
            .into_any_element()
    } else {
        glyph.into_any_element()
    };
    let task = job
        .task
        .as_ref()
        .map(|task| format!("{} · {}", read.task_key(task.id), task.title));
    let last = job.last.as_ref().or(job.file.as_ref()).map(|last| {
        if last.is_file() {
            format!("File: {}", last.text)
        } else {
            last.text.clone()
        }
    });
    let active = last_active(job).map(|at| relative(at, now));
    let (open, id) = (store.clone(), job.id.clone());
    tab_stop(
        div()
            .id(ElementId::Name(format!("job-{}", job.id).into()))
            .button_named(a11y::line(&[&job.title, &facts(job, now)]))
            .border_1()
            .rounded(px(8.))
            .flex()
            .items_start()
            .gap(px(10.))
            .p(px(8.))
            .cursor_pointer()
            .when(job.is_failed(), |d| d.bg(p.danger.opacity(0.09)))
            .hover(|d| d.bg(p.hover))
            .child(div().w(px(22.)).pt(px(1.)).child(mark))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_clamp(2)
                            .child(direction::marked(&job.title)),
                    )
                    .when_some(task, |d, task| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .truncate()
                                .child(task),
                        )
                    })
                    .when_some(last, |d, last| {
                        let color = if job.is_failed() {
                            p.danger.opacity(0.85)
                        } else {
                            p.muted
                        };
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(color)
                                .line_clamp(2)
                                .child(last),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .text_color(p.faint)
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .when(job.is_failed(), |d| d.text_color(p.danger))
                                    .child(facts(job, now)),
                            )
                            .children(active),
                    ),
            )
            .on_click(move |_, window, cx| {
                let key = open.update(cx, |store, _| store.job_key(id.clone()));
                column::push(Page::Job(key), window, cx);
            }),
        p,
    )
}

/// The row that opens the jobs over the page it is on (the Machines page): Back returns there.
pub(crate) fn link_row(p: &Pal) -> impl IntoElement {
    tab_stop(
        div()
            .id("background-jobs-row")
            .button_named("Background jobs")
            .hint("What runs on the machines now: workers, runs, services")
            .border_1()
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .p(px(12.))
            .bg(p.card)
            .border_color(p.border)
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(
                Icon::new(IconName::SquareTerminal)
                    .size(px(16.))
                    .text_color(crate::ui::palette::category_color(5)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(div().text_size(px(13.)).child("Background jobs"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child("What runs on the machines now: workers, runs, services"),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(px(13.))
                    .text_color(p.faint),
            )
            .on_click(|_, window, cx| column::push(Page::Jobs, window, cx)),
        p,
    )
}
