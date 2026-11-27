//! A turn's to-do list: how far along (a bar and `3/7`), then each item ticked, in progress (the
//! spark while the turn runs) or waiting, struck through once done. A running list of five or
//! fewer shows open; a longer one, or a finished turn's, folds to its count and the item in
//! progress until its head is clicked.

use emberhut_api::runner::steps::{TodoItem, TodoStatus};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, FontWeight, div, px, relative};

use super::spark::spark;
use super::transcript::Draw;
use crate::store::sessions::turns;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// How wide the progress bar is at most.
const BAR_WIDTH: f32 = 70.;

/// The to-do list `key` (its fold is kept by that), running (`live`) or a finished turn's.
pub(crate) fn todo_list(draw: &Draw<'_>, key: &str, todos: &[TodoItem], live: bool) -> AnyElement {
    let p = draw.p;
    let fold_key = format!("todos-{key}");
    let open = turns::todos_open(live, todos.len(), draw.open(&fold_key));
    let (done, all) = turns::todo_progress(todos);
    let now = (!open).then(|| turns::todo_now(todos)).flatten();
    let head = div()
        .id(ElementId::Name(format!("todos-head-{key}").into()))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_1()
        .rounded(px(4.))
        .cursor_pointer()
        .child(glyph(IconName::ListChecks, p.muted, 12.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Tasks"),
        )
        .child(bar(p, turns::todo_share(todos), done == all))
        .child(
            div()
                .text_size(px(11.))
                .text_color(p.muted)
                .child(format!("{done}/{all}")),
        )
        .when_some(now, |d, now| {
            d.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(now.to_owned()),
            )
        })
        .when(now.is_none(), |d| d.child(div().flex_1()))
        .child(glyph(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.faint,
            11.,
        ))
        .on_click(draw.toggle(fold_key));
    let mut list = div().flex().flex_col().gap(px(6.)).child(tab_stop(head, p));
    if open {
        let mut items = div().flex().flex_col().gap(px(4.));
        for (i, todo) in todos.iter().enumerate() {
            items = items.child(item(draw, key, i, todo, live));
        }
        list = list.child(items);
    }
    list.into_any_element()
}

/// How far along, as a thin bar: green once every item is done.
fn bar(p: &Pal, share: f32, all_done: bool) -> Div {
    div()
        .flex_none()
        .w(px(BAR_WIDTH))
        .h(px(4.))
        .rounded_full()
        .bg(p.border)
        .child(
            div()
                .h_full()
                .w(relative(share))
                .rounded_full()
                .bg(if all_done { p.success } else { p.accent }),
        )
}

/// One item: its mark, its words.
fn item(draw: &Draw<'_>, key: &str, index: usize, todo: &TodoItem, live: bool) -> AnyElement {
    let p = draw.p;
    let mark: AnyElement = match todo.status {
        TodoStatus::Completed => glyph(IconName::CircleCheck, p.success, 12.).into_any_element(),
        TodoStatus::InProgress if live => spark(p.accent, 11., draw.cx).into_any_element(),
        TodoStatus::InProgress => glyph(IconName::CircleDot, p.accent, 12.).into_any_element(),
        _ => glyph(IconName::Circle, p.faint, 12.).into_any_element(),
    };
    let done = todo.status == TodoStatus::Completed;
    div()
        .id(ElementId::Name(format!("todo-{key}-{index}").into()))
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .w(px(14.))
                .pt(px(2.))
                .flex()
                .justify_center()
                .child(mark),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .line_clamp(2)
                .text_size(px(12.))
                .when(done, |d| d.line_through().text_color(p.muted))
                .when(todo.status == TodoStatus::InProgress, |d| {
                    d.font_weight(FontWeight::SEMIBOLD)
                })
                .child(todo.content.clone()),
        )
        .into_any_element()
}
