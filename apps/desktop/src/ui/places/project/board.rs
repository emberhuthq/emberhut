//! A project page's work as a board: per lane a row of the status columns (a click on a
//! column's head folds it; Done starts folded), each card the task's key and day, its title,
//! what to do and a parent's steps.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, div, px};
use jiff::civil::Date;

use super::stage_mark;
use crate::prose::direction;
use crate::store::projects::{Item, Lane, Stage};
use crate::store::tasks::counts;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::reads;
use crate::ui::tasks::marks::{deadline_chip, glyph};

/// The board: per lane a row of the status columns, Done folded.
pub(super) fn board(
    p: &Pal,
    store: &Entity<Store>,
    read: &Store,
    lanes: &[Lane<'_>],
    today: Date,
) -> AnyElement {
    let mut rows = div().flex().flex_col().gap(px(16.)).pt(px(12.));
    for lane in lanes {
        let mut columns = div().flex().items_start().gap(px(10.));
        for stage in Stage::ALL {
            let items: Vec<&Item> = lane
                .items
                .iter()
                .filter(|item| item.stage == stage)
                .collect();
            let flipped = |key: &str| read.places().is_flipped(key);
            columns = columns.child(board_column(p, store, lane, stage, &items, &flipped, today));
        }
        rows = rows.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(direction::marked(&lane.title)),
                )
                .child(columns),
        );
    }
    div()
        .id("project-board")
        .overflow_x_scroll()
        .child(rows)
        .into_any_element()
}

/// A board column's fold key.
fn board_key(lane: &Lane<'_>, stage: Stage) -> String {
    format!("board-{}-{}", lane.title, stage.name())
}

/// A status column on the board: its name and count (a click folds it), its cards.
fn board_column(
    p: &Pal,
    store: &Entity<Store>,
    lane: &Lane<'_>,
    stage: Stage,
    items: &[&Item],
    flipped: &dyn Fn(&str) -> bool,
    today: Date,
) -> Div {
    let key = board_key(lane, stage);
    let folded = (stage == Stage::Done) != flipped(&key);
    let (flip, flip_key) = (store.clone(), key.clone());
    let mut column = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(6.))
        .w(px(if folded { 110. } else { 220. }))
        .child(
            div()
                .id(ElementId::Name(key.into()))
                // Read as the column's name and count, open or folded.
                .button_named(format!("{}, {}", stage.name(), items.len()))
                .aria_expanded(!folded)
                .aria_value(a11y::folded(!folded))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .child(stage_mark(p, stage))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(stage.name()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(items.len().to_string()),
                )
                .on_click(move |_, _, cx| {
                    flip.update(cx, |store, cx| store.flip_place_fold(&flip_key, cx));
                }),
        );
    if folded {
        return column;
    }
    for item in items {
        column = column.child(board_card(p, store, item, flipped, today));
    }
    column
}

/// A parent's steps inside its card: one line each with its stage's mark, deeper ones
/// indented; a click opens the step.
fn steps(p: &Pal, items: &[Item], depth: u16) -> Vec<AnyElement> {
    let mut lines = Vec::new();
    for item in items {
        let id = item.id;
        lines.push(
            div()
                .id(ElementId::Name(format!("step-{id}").into()))
                .button_named(a11y::item(&item.title, item.stage.name(), None))
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(f32::from(depth) * 12.))
                .text_size(px(12.))
                .text_color(if item.stage == Stage::Done {
                    p.muted
                } else {
                    p.text
                })
                .cursor_pointer()
                .child(stage_mark(p, item.stage))
                .child(div().line_clamp(2).child(direction::marked(&item.title)))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    column::push(Page::Task(id), window, cx);
                })
                .into_any_element(),
        );
        lines.extend(steps(p, &item.kids, depth + 1));
    }
    lines
}

/// A card on the project's board: the task's key and day, its title, what to do, and a
/// parent's steps (`3/5`, opening under it).
fn board_card(
    p: &Pal,
    store: &Entity<Store>,
    item: &Item,
    flipped: &dyn Fn(&str) -> bool,
    today: Date,
) -> gpui_kit::Stateful<Div> {
    let id = item.id;
    let (done, total) = item.leaves();
    let chip = counts::deadline_chip(item.day.as_ref(), false, today);
    div()
        .id(ElementId::Name(format!("project-card-{id}").into()))
        // One line for the card: its title, its stage and what to do.
        .button_named(a11y::item(
            &item.title,
            item.stage.name(),
            item.action.as_deref(),
        ))
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(p.hover)
        .cursor_pointer()
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(format!("{}{id}", crate::wire::TASK_KEY_PREFIX)),
                )
                .children(chip.map(|chip| deadline_chip(p, chip))),
        )
        .child(
            div()
                .text_size(px(13.))
                .line_clamp(3)
                .when(reads::mostly_right(&item.title), Styled::text_right)
                .child(direction::marked(&item.title)),
        )
        .children(item.action.clone().map(|action| {
            div()
                .text_size(px(12.))
                .text_color(p.accent)
                .line_clamp(2)
                .child(action)
        }))
        .when(!item.kids.is_empty(), |d| {
            let steps_key = format!("steps-{id}");
            let open = flipped(&steps_key);
            let toggle = store.clone();
            d.child(
                div()
                    .id(ElementId::Name(format!("{steps_key}-toggle").into()))
                    .button_named(format!("{done} of {total} steps done"))
                    .aria_expanded(open)
                    .aria_value(a11y::folded(open))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .child(glyph(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        p.muted,
                        9.,
                    ))
                    .child(format!(
                        "{done}/{total} {}",
                        if total == 1 { "step" } else { "steps" }
                    ))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        toggle.update(cx, |store, cx| store.flip_place_fold(&steps_key, cx));
                    }),
            )
            .when(open, |d| d.children(steps(p, &item.kids, 0)))
        })
        .on_click(move |_, window, cx| column::push(Page::Task(id), window, cx))
}
