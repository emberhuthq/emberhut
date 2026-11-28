//! The activity on a task's page that shows its plan: one feed, newest first, under a filter
//! (All · Turns · Plan · Files). A turn of work reads as one entry beside its time (the live one
//! with the amber spinner): what it came to in one line, what it was asked, its receipts (✓
//! Ticked, – Skipped, ⚠ Blocked, a file made) and its session with `Open session ›`. The plan's
//! other changes, files and the history's steps are one line each. The whole transcript is the
//! session's own page.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, FontWeight, Hsla, div, px};

use super::text::link_button;
use super::{Part, TaskView, fold, section};
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::tasks::activity::{self, Event, EventKind, Filter, Item, Turn};
use crate::store::tasks::runs::RunState;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::sessions::{self, Open};
use crate::ui::tasks::marks::glyph;
use crate::ui::tasks::plan_marks;

/// How many lines show before `Show all`.
const SHOWN: usize = 8;

/// The width of the time column.
const TIME_WIDTH: f32 = 60.;

/// The activity, when the task has any.
pub(super) fn section_of(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    if activity::feed(part.page, part.now, Filter::All).is_empty() {
        return body;
    }
    let p = part.p;
    let items = activity::feed(part.page, part.now, view.feed);
    let note = activity::sessions_note(&items);
    body = body.child(
        div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(section(p, "Activity").pt(px(20.)))
            .child(div().flex_1())
            .children(note.map(|note| {
                div()
                    .pb(px(6.))
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(note)
            })),
    );
    body = body.child(filters(p, view.feed, cx));
    if items.is_empty() {
        return body.child(
            div()
                .py(px(8.))
                .text_size(px(12.5))
                .text_color(p.muted)
                .child("Nothing here yet."),
        );
    }
    let shown = if view.is_open(fold::ALL_RUNS) {
        items.len()
    } else {
        SHOWN.min(items.len())
    };
    for item in items.iter().take(shown) {
        body = body.child(match item {
            Item::Turn(turn) => turn_row(part, turn, cx),
            Item::Event(event) => event_row(part, event).into_any_element(),
        });
    }
    if items.len() > shown {
        body = body.child(
            div().pt(px(6.)).child(
                link_button(
                    p,
                    "all-activity".into(),
                    format!("Show all {}", items.len()),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_open(fold::ALL_RUNS);
                    cx.notify();
                })),
            ),
        );
    }
    body
}

/// The filter's choices, the one picked lit.
fn filters(p: &Pal, picked: Filter, cx: &Context<'_, TaskView>) -> gpui_kit::Stateful<Div> {
    let mut row = div()
        .id("activity-filter")
        .read_as(gpui_kit::Role::RadioGroup, "Show in the activity")
        .flex()
        .gap(px(4.))
        .pb(px(8.));
    for filter in Filter::ALL {
        let on = filter == picked;
        row = row.child(tab_stop(
            div()
                .id(ElementId::Name(
                    format!("activity-{}", filter.label()).into(),
                ))
                .read_as(gpui_kit::Role::RadioButton, filter.label())
                .aria_selected(on)
                .border_1()
                .rounded(px(6.))
                .px(px(8.))
                .py(px(2.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| {
                    d.bg(p.accent_soft)
                        .text_color(p.accent)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!on, |d| d.text_color(p.muted).hover(|d| d.bg(p.hover)))
                .child(filter.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.feed = filter;
                    cx.notify();
                })),
            p,
        ));
    }
    row
}

/// A turn's colour: amber while it runs, else its STATUS line's.
fn color(p: &Pal, turn: &Turn) -> Hsla {
    if turn.live {
        return p.warn;
    }
    match turn.state {
        Some(RunState::Done) => p.success,
        Some(RunState::Working) => p.info,
        Some(RunState::Blocked) => p.danger,
        Some(RunState::NeedsYou) => p.accent,
        None => p.muted,
    }
}

/// The time column: the clock time today, else the day.
fn time(part: &Part<'_>, at: Option<jiff::Timestamp>) -> Div {
    div()
        .w(px(TIME_WIDTH))
        .flex_none()
        .pt(px(1.))
        .text_size(px(11.5))
        .text_color(part.p.faint)
        .children(at.map(|at| activity::time_label(at, part.now)))
}

/// A turn: when, what it came to, what it was asked, its receipts and its session.
fn turn_row(part: &Part<'_>, turn: &Turn, cx: &Context<'_, TaskView>) -> AnyElement {
    let p = part.p;
    let said = div()
        .flex()
        .items_start()
        .gap(px(6.))
        .when(turn.live, |d| {
            d.child(div().pt(px(3.)).child(plan_marks::running(
                p,
                ElementId::Name(format!("activity-live-{}", turn.run).into()),
                11.,
                cx,
            )))
        })
        .when(!turn.live && turn.state.is_some(), |d| {
            d.child(
                div()
                    .pt(px(6.))
                    .child(crate::ui::widgets::dot(color(p, turn), 6.)),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(if turn.live { p.muted } else { p.text })
                .line_clamp(3)
                .children(turn.said.clone()),
        );
    let mut text = div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .flex_1()
        .min_w_0()
        .child(said);
    if let Some(asked) = &turn.asked {
        text = text.child(
            div()
                .truncate()
                .text_size(px(11.5))
                .text_color(p.faint)
                .child(format!("Asked: {asked}")),
        );
    }
    if !turn.receipts.is_empty() {
        text = text.child(
            div()
                .flex()
                .flex_wrap()
                .gap_x(px(12.))
                .gap_y(px(2.))
                .children(turn.receipts.iter().map(|event| receipt(p, event))),
        );
    }
    if let Some((session, label)) = turn.session.clone() {
        let place = SessionPlace::Session(SessionRef::whole(session));
        let store = part.store.clone();
        text = text.child(tab_stop(
            div()
                .id(ElementId::Name(
                    format!("activity-session-{}", turn.run).into(),
                ))
                .button_named(format!("Open session: {label}"))
                .hint("Opens the session this turn ran in")
                .border_1()
                .rounded(px(5.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.5))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(glyph(IconName::SquareTerminal, p.faint, 12.))
                .child(div().text_color(p.muted).child(label))
                .child(div().text_color(p.faint).child("·"))
                .child(div().text_color(p.accent).child("Open session ›"))
                .on_click(move |_, window, cx| {
                    sessions::push(&store, Open::Place(place.clone()), window, cx);
                }),
            p,
        ));
    }
    div()
        .id(ElementId::Name(format!("activity-{}", turn.run).into()))
        .read_as(
            gpui_kit::Role::Group,
            a11y::line(&[
                &turn.when,
                turn.said.as_deref().unwrap_or_default(),
                turn.asked.as_deref().unwrap_or_default(),
            ]),
        )
        .flex()
        .gap(px(8.))
        .py(px(7.))
        .child(time(part, turn.at))
        .child(text)
        .into_any_element()
}

/// A receipt under a turn: `✓ Ticked: <step>` and the like, in its colour.
fn receipt(p: &Pal, event: &Event) -> Div {
    let (icon, color) = mark(p, event.kind);
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.5))
        .text_color(color)
        .child(glyph(icon, color, 11.))
        .child(match event.kind.word() {
            Some(word) => format!("{word}: {}", event.text),
            None => event.text.clone(),
        })
}

/// An event's icon and colour.
fn mark(p: &Pal, kind: EventKind) -> (IconName, Hsla) {
    match kind {
        EventKind::Ticked => (IconName::Check, p.success),
        EventKind::Skipped => (IconName::Minus, p.faint),
        EventKind::Blocked => (IconName::TriangleAlert, p.danger),
        EventKind::File => (IconName::File, p.muted),
        EventKind::Note => (IconName::Circle, p.faint),
    }
}

/// An event on its own line beside its time.
fn event_row(part: &Part<'_>, event: &Event) -> Div {
    let p = part.p;
    let (icon, color) = mark(p, event.kind);
    div()
        .flex()
        .gap(px(8.))
        .py(px(4.))
        .child(time(part, Some(event.at)))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .child(div().pt(px(2.)).child(glyph(icon, color, 11.)))
                .children(event.kind.word().map(|word| {
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text)
                        .child(format!("{word}:"))
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(p.muted)
                        .line_clamp(2)
                        .child(event.text.clone()),
                ),
        )
}
