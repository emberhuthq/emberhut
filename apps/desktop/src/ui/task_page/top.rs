//! The top of a task's page: its title, what is happening right now (a capsule with its state
//! and for how long, its ETA, the action when it waits on the user or who is on it, then where
//! or when it runs, what it waits on, what the open run was asked and when the last one
//! ended), then the decision (`decision.rs`), and where it stands.

use emberhut_api::task_live::LiveState;
use emberhut_api::tasks::TaskStatus;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, div, px};

use super::text::{Long, collapsible, rich};
use super::{Part, TaskView, section};
use crate::brand::APP_NAME;
use crate::store::tasks::{clock, runs, words};
use crate::ui::reads::reads;
use crate::ui::tasks::marks::{status_icon, tone_color};

/// The title, the live card and the decision.
pub(super) fn title_and_state(
    body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    asks(title_and_live(body, part), part, view, cx)
}

/// The title and the live card.
pub(super) fn title_and_live(body: Div, part: &Part<'_>) -> Div {
    body.child(
        reads(&part.task.title)
            .pt(px(14.))
            .text_size(px(18.))
            .font_weight(FontWeight::BOLD),
    )
    .child(live_card(part))
}

/// What the task asks of the user: an X post to look at, and the decision.
pub(super) fn asks(body: Div, part: &Part<'_>, view: &TaskView, cx: &Context<'_, TaskView>) -> Div {
    let body = super::decision::x_posts(body, part, cx);
    super::decision::section_of(body, part, view, cx)
}

/// Right now: the state and how long, the ETA, the action or who is on it, and the lines
/// under them.
fn live_card(part: &Part<'_>) -> Div {
    let (p, task, now) = (part.p, part.task, part.now);
    let live = task.live.as_ref();
    let runs = runs::timeline(part.page);
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .pt(px(10.))
        .child(capsule_row(part));
    let doing = words::doing(task);
    let running = matches!(doing, words::Doing::Running | words::Doing::Queued);
    if task.status == TaskStatus::Waiting
        && let Some(action) = &task.action
    {
        card = card.child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(rich("action", action, None, part.store)),
        );
    } else if let Some(run) = live.and_then(|live| live.run.as_ref()).filter(|_| running)
        && (run.worker || run.title.is_some())
    {
        let who = match (&run.title, run.worker) {
            (_, true) => format!("{APP_NAME}'s worker is on it"),
            (Some(title), false) if doing == words::Doing::Queued => {
                format!("Waiting to start on “{title}”")
            }
            (Some(title), false) => format!("Working on “{title}”"),
            (None, false) => String::new(),
        };
        card = card.child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(who),
        );
    }
    let scheduled = live.is_some_and(|live| live.state == LiveState::Scheduled);
    let key_of = |id| part.read.task_key(id);
    let mut lines: Vec<(&'static str, String)> = Vec::new();
    if let Some(line) = words::live_line(task, now, &key_of) {
        lines.push((if scheduled { "When" } else { "Where" }, line));
    }
    if scheduled && let Some(live) = live {
        let waits = live
            .watch
            .as_ref()
            .and_then(|watch| watch.title.clone())
            .or_else(|| {
                live.checkin
                    .as_ref()
                    .map(|checkin| without_key(&checkin.why))
            });
        if let Some(waits) = waits {
            lines.push(("Waits on", waits));
        }
    }
    if let Some(asked) = runs
        .iter()
        .find(|run| run.ended.is_none())
        .and_then(|run| run.asked.clone())
    {
        lines.push(("Asked", asked));
    }
    if let Some(last) = runs::last_run_line(&runs, now) {
        lines.push(("Last run", last));
    }
    for (label, value) in lines {
        card = card.child(
            div()
                .flex()
                .gap(px(12.))
                .text_size(px(12.5))
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_color(p.muted)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .line_clamp(if label == "When" { 4 } else { 2 })
                        .child(value),
                ),
        );
    }
    card
}

/// The state's capsule (its icon, its name and how long) and the ETA.
fn capsule_row(part: &Part<'_>) -> Div {
    let (p, task) = (part.p, part.task);
    let color = tone_color(p, words::tone(task));
    let since = task
        .live
        .as_ref()
        .and_then(|live| live.since.as_ref())
        .and_then(clock::instant)
        .map(|since| clock::span(since, part.now.timestamp()));
    let capsule = [Some(words::name(task)), since]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let eta = task
        .eta
        .clone()
        .filter(|_| !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled));
    div()
        .flex()
        .items_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(color.opacity(0.15))
                .text_color(color)
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(status_icon(p, task))
                .child(capsule),
        )
        .child(div().flex_1())
        .children(eta.map(|eta| {
            div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(format!("ETA {eta}"))
        }))
}

/// A check-in's reason without the task key it starts with (`T-4: call back` reads `call
/// back`).
fn without_key(why: &str) -> String {
    let trimmed = why.trim_start_matches(|c: char| !c.is_alphanumeric());
    let key_end = trimmed
        .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-'))
        .unwrap_or(trimmed.len());
    let key = &trimmed[..key_end];
    if key.contains('-') && key.ends_with(|c: char| c.is_ascii_digit()) {
        trimmed[key_end..]
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .to_owned()
    } else {
        why.to_owned()
    }
}

/// Where it stands: the latest answer, folded to four lines, and what is left.
pub(super) fn standing(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let runs = runs::timeline(part.page);
    let Some(standing) = runs::standing(part.task, part.page, &runs) else {
        return body;
    };
    let p = part.p;
    body = body.child(section(p, "Where it stands")).child(collapsible(
        p,
        part.store,
        Long {
            key: "standing",
            text: &standing.line,
            lines: 4,
            title: None,
        },
        view,
        cx,
    ));
    if let Some(left) = standing.left {
        body = body.child(
            div()
                .flex()
                .gap(px(4.))
                .pt(px(6.))
                .text_size(px(13.))
                .child(div().text_color(p.muted).child("Left:"))
                .child(left),
        );
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_ins_reason_loses_its_key() {
        assert_eq!(without_key("T-4: call the bank back"), "call the bank back");
        assert_eq!(without_key("Call the bank"), "Call the bank");
    }
}
