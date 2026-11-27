//! Small pieces every page draws with.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, ScrollHandle, SharedString, Stateful, div, px, relative};

use super::palette::Pal;

/// A round dot.
pub(crate) fn dot(color: Hsla, size: f32) -> Div {
    div().size(px(size)).rounded_full().bg(color).flex_none()
}

/// A small rounded label: a count, a state, a chip.
pub(crate) fn pill(text: impl Into<SharedString>, background: Hsla, foreground: Hsla) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(8.))
        .bg(background)
        .text_color(foreground)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .flex_none()
        .child(text.into())
}

/// An icon in a colour and size.
pub(crate) fn icon(name: IconName, color: Hsla, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

/// A labelled bar: its name, the numbers, a thin capsule filled to its share, a note.
pub(crate) fn bar(
    p: &Pal,
    label: impl Into<SharedString>,
    share: f64,
    numbers: impl Into<SharedString>,
    note: Option<String>,
    color: Option<Hsla>,
) -> Div {
    let width = self::share(share);
    let fill = color.unwrap_or(if width >= 0.9 {
        p.danger
    } else if width >= 0.75 {
        p.warn
    } else {
        p.info
    });
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .py(px(3.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.into()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child(numbers.into()),
                ),
        )
        .child(
            div()
                .h(px(6.))
                .w_full()
                .rounded_full()
                .bg(p.border)
                .child(div().h_full().rounded_full().bg(fill).w(relative(width))),
        )
        .when_some(note.filter(|note| !note.is_empty()), |d, note| {
            d.child(div().text_size(px(11.5)).text_color(p.muted).child(note))
        })
}

/// A small section title in a list.
pub(crate) fn section_title(p: &Pal, title: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.faint)
        .px(px(10.))
        .pt(px(14.))
        .pb(px(4.))
        .child(title.into())
}

/// A page's title bar, with an optional line beside the title.
pub(crate) fn page_head(p: &Pal, title: &'static str, subtitle: Option<String>) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(10.))
        .h(px(52.))
        .px(px(24.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .pt(px(16.))
                .child(title),
        )
        .when_some(subtitle, |d, subtitle| {
            d.child(div().text_size(px(12.)).text_color(p.muted).child(subtitle))
        })
}

/// A page's scrolling body.
pub(crate) fn scroller(id: &'static str, handle: &ScrollHandle) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .overflow_y_scroll()
        .track_scroll(handle)
}

/// A rounded group of settings, titled.
pub(crate) fn group(p: &Pal, title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(16.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title),
        )
}

/// A share as GPUI's relative sizes take it: clamped to 0..=1, and 0 when it isn't a number.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "a share between 0 and 1 fits an f32"
)]
pub(crate) fn share(value: f64) -> f32 {
    if value.is_finite() {
        value.clamp(0., 1.) as f32
    } else {
        0.
    }
}
