//! The work on a task's page: its subtasks with their rollup, a finding's state (and what found
//! and fixes it) or a scan's findings grouped by state, every run newest first, and its sessions
//! on the runners.

use emberhut_api::ids::TaskId;
use emberhut_api::task_page::Run;
use emberhut_api::tasks::TaskStatus;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, FontWeight, ImageSource, div, img, px};

use super::text::{Long, collapsible, link_button};
use super::{Part, TaskView, fold, fold_row, section};
use crate::store::Page;
use crate::store::files::RemoteFile;
use crate::store::tasks::runs::{self, RunState, RunStatus};
use crate::store::tasks::{counts, findings};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::{glyph, landed_mark, progress};
use crate::ui::tasks::row::{self, Look};
use crate::ui::viewer::OpenInViewer;

/// How many runs show before `Show all`.
const RUNS_SHOWN: usize = 5;

/// A parent's subtasks: `7/21 done · 160/300 episodes` with a bar, then each one (archived ones
/// left out), in order, opening its page.
pub(super) fn subtasks(mut body: Div, part: &Part<'_>) -> Div {
    let Some(rollup) = &part.task.sub else {
        return body;
    };
    let p = part.p;
    let fill = if rollup.waiting.is_empty() {
        p.info
    } else {
        p.accent
    };
    body = body
        .child(section(p, "Subtasks"))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(counts::rollup_summary(rollup)),
        )
        .child(progress(
            p,
            counts::fraction(part.task).unwrap_or(0.0),
            fill,
        ));
    let today = part.now.date();
    for subtask in part
        .page
        .map_or(&[][..], |page| &page.subtasks)
        .iter()
        .filter(|subtask| subtask.status != TaskStatus::Canceled)
    {
        body = body.child(row::row(
            part.store,
            p,
            subtask,
            Look {
                project: None,
                today,
                health: None,
                open: false,
                nested: false,
                swipes: None,
                ticked: part.read.is_ticked(subtask.id),
            },
        ));
    }
    body
}

/// A link to another task's page, by its key.
fn task_link(p: &Pal, id: &'static str, key: String, task: TaskId) -> gpui_kit::Stateful<Div> {
    link_button(p, id.into(), key).on_click(move |_, window, cx| {
        column::push(Page::Task(task), window, cx);
    })
}

/// A finding: whether it landed, what found it and what fixes it. A scan or review: its
/// findings, the count of each state, opening them grouped by state with what each means.
pub(super) fn finding(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let p = part.p;
    let task = part.page.map_or(part.task, |page| &page.task);
    if task.is_finding() {
        let landed = findings::landed(task);
        body = body.child(section(p, "Finding")).child(
            div()
                .id("finding-state")
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .child(div().w(px(70.)).text_color(p.muted).child("State"))
                .child(landed_mark(p, &landed))
                .child(findings::name(&landed))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(findings::meaning(&landed))
                        .build(window, cx)
                }),
        );
        let lines = [
            ("Found by", "found-by", task.parent),
            ("Fixed by", "fixed-by", task.fixed_by),
        ];
        for (label, id, other) in lines {
            if let Some(other) = other {
                body = body.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .pt(px(4.))
                        .text_size(px(13.))
                        .child(div().w(px(70.)).text_color(p.muted).child(label))
                        .child(task_link(p, id, part.read.task_key(other), other)),
                );
            }
        }
    }
    findings_list(body, part, view, cx)
}

/// A scan's or a review's findings: a row with the count of each state, opening them grouped
/// by state with what each state means.
fn findings_list(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let p = part.p;
    let Some(found) = part
        .page
        .map(|page| &page.findings)
        .filter(|found| !found.is_empty())
    else {
        return body;
    };
    let groups = findings::grouped(found);
    let mut counts = div().flex().gap(px(8.));
    for (state, rows) in &groups {
        counts = counts.child(
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .text_size(px(12.))
                .text_color(p.muted)
                .child(landed_mark(p, state))
                .child(rows.len().to_string()),
        );
    }
    let open = view.is_open(fold::FINDINGS);
    body = body.child(
        fold_row(p, "findings", "Findings".to_owned(), open)
            .child(counts)
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_open(fold::FINDINGS);
                cx.notify();
            })),
    );
    if !open {
        return body;
    }
    let mut legend = div().flex().flex_col().gap(px(2.)).py(px(4.));
    for state in &findings::ORDER {
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(p.faint)
                .child(landed_mark(p, state))
                .child(findings::meaning(state)),
        );
    }
    body = body.child(legend);
    let today = part.now.date();
    for (state, rows) in groups {
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pt(px(10.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(landed_mark(p, &state))
                .child(findings::name(&state))
                .child(div().text_color(p.muted).child(rows.len().to_string())),
        );
        for finding in rows {
            body = body.child(row::row(
                part.store,
                p,
                finding,
                Look {
                    project: None,
                    today,
                    health: None,
                    open: false,
                    nested: false,
                    swipes: None,
                    ticked: part.read.is_ticked(finding.id),
                },
            ));
        }
    }
    body
}

/// A run's colour: orange while it runs, else its STATUS line's.
fn run_color(p: &Pal, run: &Run) -> gpui_kit::Hsla {
    if run.ended.is_none() {
        return p.warn;
    }
    match run
        .status
        .as_deref()
        .and_then(RunStatus::parse)
        .map(|status| status.state)
    {
        Some(RunState::Done) => p.success,
        Some(RunState::Working) => p.info,
        Some(RunState::Blocked) => p.danger,
        Some(RunState::NeedsYou) => p.accent,
        None => p.muted,
    }
}

/// Every run, newest first: when and how long, what it was asked, its answer, its STATUS, its
/// pictures; five, then `Show all n runs`.
pub(super) fn runs(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let all = runs::timeline(part.page);
    if all.is_empty() {
        return body;
    }
    let p = part.p;
    body = body.child(section(p, "Runs"));
    let shown = if view.is_open(fold::ALL_RUNS) {
        all.len()
    } else {
        RUNS_SHOWN.min(all.len())
    };
    let artifacts = part
        .page
        .map_or(&part.task.artifacts[..], |page| &page.task.artifacts);
    for (i, run) in all.iter().take(shown).enumerate() {
        body = body.child(run_row(part, run, i + 1 == shown, artifacts, view, cx));
    }
    if all.len() > shown {
        body = body.child(div().pt(px(6.)).child(
            link_button(p, "all-runs".into(), format!("Show all {} runs", all.len())).on_click(
                cx.listener(|this, _, _, cx| {
                    this.set_open(fold::ALL_RUNS);
                    cx.notify();
                }),
            ),
        ));
    }
    body
}

/// One run on the timeline: its dot (the colour of its STATUS) on the line down the runs, when
/// and how long, what it was asked, its answer, its STATUS, its pictures and videos.
fn run_row(
    part: &Part<'_>,
    run: &Run,
    last: bool,
    artifacts: &[emberhut_api::tasks::Artifact],
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let p = part.p;
    let color = run_color(p, run);
    let mut text = div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .flex_1()
        .min_w_0()
        .pb(px(if last { 0. } else { 14. }))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(runs::run_when(run, part.now)),
        );
    if let Some(asked) = &run.asked {
        text = text.child(
            div()
                .truncate()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(format!("Asked: {asked}")),
        );
    }
    match run
        .answer
        .as_deref()
        .filter(|answer| !answer.trim().is_empty())
    {
        Some(answer) => {
            let key = format!("run-{}", run.id);
            text = text.child(collapsible(
                p,
                part.store,
                Long {
                    key: &key,
                    text: answer,
                    lines: 2,
                    title: None,
                },
                view,
                cx,
            ));
        }
        None if run.ended.is_none() => {
            let waiting = if run.started.is_none() {
                "Not started yet"
            } else {
                "Working on it…"
            };
            text = text.child(div().text_size(px(12.5)).text_color(p.faint).child(waiting));
        }
        None => {}
    }
    if let Some(status) = run.status.as_deref().and_then(RunStatus::parse) {
        text = text.child(
            div()
                .px(px(7.))
                .py(px(2.))
                .rounded(px(6.))
                .bg(color.opacity(0.15))
                .text_color(color)
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .child(status.text),
        );
    } else if let Some(outcome) = run
        .outcome
        .as_deref()
        .filter(|outcome| *outcome != "completed")
    {
        text = text.child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.danger)
                .child(format!("Turn {outcome}")),
        );
    }
    text = text.children(run_media(part, run, artifacts));
    let dot = div()
        .size(px(12.))
        .mt(px(3.))
        .rounded_full()
        .border_2()
        .border_color(color)
        .when(run.ended.is_none(), |d| d.bg(color));
    let rail = div()
        .flex()
        .flex_col()
        .items_center()
        .w(px(12.))
        .child(dot)
        .when(!last, |d| d.child(div().w(px(2.)).flex_1().bg(p.border)));
    div().flex().gap(px(12.)).child(rail).child(text)
}

/// A run's pictures and videos as thumbnails, each opening in the viewer.
fn run_media(
    part: &Part<'_>,
    run: &Run,
    artifacts: &[emberhut_api::tasks::Artifact],
) -> Option<Div> {
    let p = part.p;
    let media = runs::media(run, artifacts, part.now);
    if media.is_empty() {
        return None;
    }
    let mut thumbs = div().flex().flex_wrap().gap(px(8.)).pt(px(4.));
    for artifact in media {
        let picture = artifact
            .thumb
            .as_deref()
            .and_then(|thumb| part.read.picture(thumb));
        let (url, name) = (artifact.url.clone(), artifact.name.clone());
        thumbs = thumbs.child(
            div()
                .id(ElementId::Name(
                    format!("run-media-{}-{}", run.id, artifact.id).into(),
                ))
                .relative()
                .w(px(120.))
                .h(px(84.))
                .rounded(px(8.))
                .bg(p.card)
                .cursor_pointer()
                .children(picture.map(|image| {
                    img(ImageSource::Render(image))
                        .w(px(120.))
                        .h(px(84.))
                        .rounded(px(8.))
                }))
                .when(artifact.kind == "video", |d| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(glyph(IconName::CirclePlay, p.on_accent, 28.)),
                    )
                })
                .on_click(move |_, window, cx| {
                    let file = RemoteFile::new(url.clone(), name.clone());
                    window.dispatch_action(Box::new(OpenInViewer { file }), cx);
                }),
        );
    }
    Some(thumbs)
}

/// The task's sessions on the runners: each one's state, title, where it runs and what it does.
pub(super) fn sessions(mut body: Div, part: &Part<'_>) -> Div {
    let Some(sessions) = part
        .page
        .map(|page| &page.sessions)
        .filter(|s| !s.is_empty())
    else {
        return body;
    };
    let p = part.p;
    body = body.child(section(p, "Sessions"));
    for session in sessions {
        let running = session.status == "running";
        let waits = session.ask.is_some();
        let color = if waits {
            p.accent
        } else if running {
            p.success
        } else {
            p.muted
        };
        let line = [
            Some(crate::format::capitalized(&session.status)),
            (!session.place.is_empty()).then(|| session.place.clone()),
            session
                .ask
                .clone()
                .or_else(|| session.doing.clone())
                .or_else(|| session.last.clone()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let place = crate::store::sessions::places::SessionPlace::Session(
            crate::store::sessions::view::SessionRef::whole(session.id.clone()),
        );
        let id = ElementId::Name(format!("task-session-{}", session.id).into());
        let row = div()
            .flex()
            .items_start()
            .gap(px(10.))
            .py(px(5.))
            .px(px(4.))
            .child(div().pt(px(5.)).child(crate::ui::widgets::dot(color, 8.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(p.muted)
                            .child(line),
                    ),
            );
        body = body.child(crate::ui::sessions::link(part.store, place, id, row, p));
    }
    body
}
