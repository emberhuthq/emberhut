//! Needs you as a board: a column per project (the one with the most urgent ask first, Other
//! last, each card tagged Today / This week / This month), then Later folded with each card's
//! project; or a column per when (Today with the overdue ones, This week, This month, Later
//! folded). A column's head (its dot, name and count) folds it to a narrow strip.

use emberhut_api::work::WorkRow;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, Hsla, div, px};

use super::{MainWindow, Shown, card_id, open_task};
use crate::prose::direction;
use crate::store::Store;
use crate::store::lookup;
use crate::store::need_card::CardContext;
use crate::store::needs_view::{ColumnKey, Grouping};
use crate::store::tasks::filter::When;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::{Pal, room_color};
use crate::ui::tasks::marks::glyph;
use crate::ui::widgets::dot;

/// A column's width, open.
const COLUMN_WIDTH: f32 = 264.;

/// A column's width, folded.
const FOLDED_WIDTH: f32 = 120.;

/// One column of the board.
struct BoardColumn<'a> {
    key: ColumnKey,
    title: String,
    color: Hsla,
    rows: Vec<&'a WorkRow>,
    /// Its cards name their project (Later's, and the columns by when).
    project: bool,
    /// Its cards' chips name their tier (by project, not Later).
    tiers: bool,
}

/// The board, scrolling sideways.
pub(super) fn board(
    p: &Pal,
    store: &Entity<Store>,
    shown: &Shown<'_>,
    grouping: Grouping,
    cx: &Context<'_, MainWindow>,
) -> AnyElement {
    let columns = columns(p, shown, grouping);
    let view = store.read(cx).needs_view();
    let mut row = div().flex().items_start().gap(px(14.)).p(px(14.));
    for column in columns {
        let folded = view.is_folded(&column.key);
        let cards = shown.cards(column.project, column.tiers);
        row = row.child(board_column(p, store, &column, folded, &cards));
    }
    div()
        .id("needs-board")
        .flex_1()
        .size_full()
        .overflow_scroll()
        .child(row)
        .into_any_element()
}

/// The board's columns over what the filter keeps.
fn columns<'a>(p: &Pal, shown: &Shown<'a>, grouping: Grouping) -> Vec<BoardColumn<'a>> {
    let later = shown.needs.later_of(shown.kept.clone());
    match grouping {
        Grouping::Project => {
            let mut columns: Vec<BoardColumn<'a>> = shown
                .needs
                .by_project_of(shown.kept.clone())
                .into_iter()
                .map(|group| BoardColumn {
                    key: ColumnKey::Project(group.project),
                    color: if group.project.is_some() {
                        p.accent
                    } else {
                        p.accent.opacity(0.5)
                    },
                    title: group.name,
                    rows: group.rows,
                    project: false,
                    tiers: true,
                })
                .collect();
            if !later.is_empty() {
                columns.push(BoardColumn {
                    key: ColumnKey::When(When::Later),
                    title: When::Later.name().to_owned(),
                    color: p.faint,
                    rows: later,
                    project: true,
                    tiers: false,
                });
            }
            columns
        }
        Grouping::When => {
            let mut by_when = shown.needs.by_when_of(&shown.kept);
            When::ALL
                .into_iter()
                .map(|when| {
                    let rows = if when == When::Later {
                        later.clone()
                    } else {
                        by_when
                            .iter_mut()
                            .find(|(of, _)| *of == when)
                            .map_or_default(|(_, rows)| std::mem::take(rows))
                    };
                    BoardColumn {
                        key: ColumnKey::When(when),
                        title: when.name().to_owned(),
                        color: match when {
                            When::Today => p.accent,
                            When::Later => p.faint,
                            When::ThisWeek | When::ThisMonth => p.accent.opacity(0.5),
                        },
                        rows,
                        project: !shown.hides_projects(),
                        tiers: false,
                    }
                })
                .collect()
        }
    }
}

/// A column: its head (a click folds or opens it) and, open, its cards.
fn board_column(
    p: &Pal,
    store: &Entity<Store>,
    column: &BoardColumn<'_>,
    folded: bool,
    cards: &CardContext<'_>,
) -> Div {
    let key = column.key.clone();
    let toggle = store.clone();
    let title = column.title.clone();
    let head = div()
        .id(ElementId::Name(format!("board-head-{title}").into()))
        // Read as the column's name and count, open or folded.
        .button_named(format!("{title}, {}", column.rows.len()))
        .aria_expanded(!folded)
        .aria_value(a11y::folded(!folded))
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(if folded {
                format!("Show {title}")
            } else {
                format!("Fold {title}")
            })
            .build(window, cx)
        })
        .child(dot(column.color, 7.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(column.title.clone()),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(column.rows.len().to_string()),
        )
        .when(folded, |d| {
            d.child(glyph(IconName::ChevronRight, p.faint, 10.))
        })
        .on_click(move |_, _, cx| {
            let key = key.clone();
            toggle.update(cx, |store, cx| store.toggle_needs_column(key, cx));
        });
    let mut body = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(6.))
        .w(px(if folded { FOLDED_WIDTH } else { COLUMN_WIDTH }))
        .child(head);
    if folded {
        return body;
    }
    if column.rows.is_empty() {
        return body.child(
            div()
                .py(px(4.))
                .text_size(px(12.))
                .text_color(p.faint)
                .child("Nothing"),
        );
    }
    for row in &column.rows {
        body = body.child(board_card(p, store, row, cards));
    }
    body
}

/// A card on the board: the task's key and chip, its title, what to do, and its project (with
/// its goal's colour) where the column doesn't say it.
fn board_card(
    p: &Pal,
    store: &Entity<Store>,
    row: &WorkRow,
    cards: &CardContext<'_>,
) -> AnyElement {
    let shown = cards.card(row);
    let key = row.task.map_or_default(|task| cards.needs.key(task));
    let goal = lookup::goal_rank(
        cards.state,
        row.pid
            .and_then(|pid| {
                cards
                    .state
                    .projects
                    .iter()
                    .find(|project| project.id == pid)
            })
            .and_then(|project| project.goal),
    );
    let mut card = div()
        .id(card_id("board", row))
        // One line for the card: its title, what to do and its key (a button when it opens).
        .read_as(
            if row.task.is_some() {
                gpui_kit::Role::Button
            } else {
                gpui_kit::Role::Group
            },
            a11y::line(&[
                &row.title,
                row.action
                    .as_deref()
                    .or(row.ask.as_deref())
                    .unwrap_or_default(),
                &key,
            ]),
        )
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
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(key),
                )
                .children(
                    shown
                        .chip
                        .as_ref()
                        .map(|chip| super::card::decide_chip(p, chip)),
                ),
        )
        .child(
            div()
                .text_size(px(13.))
                .line_clamp(3)
                .when(
                    crate::ui::reads::mostly_right(&row.title),
                    Styled::text_right,
                )
                .child(direction::marked(&row.title)),
        )
        .children(row.action.clone().or_else(|| row.ask.clone()).map(|what| {
            div()
                .text_size(px(12.))
                .text_color(p.accent)
                .line_clamp(2)
                .child(what)
        }))
        .children(shown.project.map(|project| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(room_color(goal, p), 6.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(project.to_owned()),
                )
        }));
    let Some(task) = row.task else {
        return super::thread_card(card, store, row, p);
    };
    card = tab_stop(card, p).on_click(move |_, window, cx| open_task(task, window, cx));
    super::card::with_menu(card, store, task).into_any_element()
}
