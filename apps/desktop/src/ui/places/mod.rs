//! The pages a project, a goal, a person and the week open on, in the column beside the chat:
//! a project's page (`project/`), a goal's (`goal.rs`), a person's (`person.rs`) and the
//! weekly report (`report.rs`), with the small pieces they share.

mod goal;
mod person;
mod project;
mod report;
pub(crate) mod ring;

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, FontWeight, ScrollHandle, div, px};

use super::palette::{Pal, pal};
use super::reads::reads;
use super::widgets::scroller;
use super::window::MainWindow;
use crate::brand::APP_NAME;
use crate::store::Page;

/// The page for `page`.
pub(crate) fn page(
    window: &mut MainWindow,
    page: Page,
    scroll: &ScrollHandle,
    cx: &mut Context<'_, MainWindow>,
) -> AnyElement {
    let p = pal(cx);
    let body = match page {
        Page::Project(project) => project::page(window, &p, project, cx),
        Page::Goal(goal) => goal::page(window, &p, goal, cx),
        Page::Person(person) => person::page(window, &p, person, cx),
        _ => report::page(window, &p, cx),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(scroller("place-scroll", scroll).child(body))
        .into_any_element()
}

/// A page's title, from the right edge when most of its letters are Arabic.
fn title(text: impl Into<gpui_kit::SharedString>) -> Div {
    reads(&text.into())
        .text_size(px(18.))
        .font_weight(FontWeight::BOLD)
}

/// A section's title.
fn section(p: &Pal, text: impl Into<gpui_kit::SharedString>) -> Div {
    div()
        .pt(px(16.))
        .pb(px(6.))
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.faint)
        .child(text.into())
}

/// A grey line where the page would be.
fn gone(p: &Pal, text: &'static str) -> Div {
    div()
        .pt(px(80.))
        .flex()
        .justify_center()
        .text_size(px(14.))
        .text_color(p.muted)
        .child(text)
}

/// Who took a step: `You` in blue, the assistant in orange.
fn who(p: &Pal, owner: &str) -> Div {
    let mine = owner != crate::wire::ASSISTANT_OWNER;
    let color = if mine { p.info } else { p.warn };
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded_full()
        .bg(color.opacity(0.18))
        .text_color(color)
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(if mine {
            "You".to_owned()
        } else {
            APP_NAME.to_owned()
        })
}

/// A page's body, padded.
fn body() -> Div {
    div().flex().flex_col().gap(px(4.)).px(px(20.)).py(px(14.))
}
