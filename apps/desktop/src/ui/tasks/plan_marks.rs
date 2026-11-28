//! The marks a plan is drawn with, on a server that keeps plans: running is always amber and
//! always a shape (a ring with an arc, turned once a second by the view that shows it), blocked a red disc with `!`,
//! waiting on the user a filled ember dot; and the bar, one segment a step (done filled, doing
//! amber, blocked red, to do empty) or one plain fill for a long plan.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, FontWeight, Transformation, div, percentage, px};

use super::marks::{glyph, progress};
use crate::store::tasks::plan::{Bar, Lead, Segment};
use crate::ui::palette::Pal;

/// The running mark: an amber ring with its arc, `size` across, turned a quarter for each
/// `turn`. It never animates by frame: a view that shows it moves `turn` once a second while
/// something runs (and not under Reduce motion), so a long list redraws once a second, not
/// every frame.
pub(crate) fn running(p: &Pal, size: f32, turn: u8) -> Div {
    let arc = glyph(IconName::LoaderCircle, p.warn, size + 2.)
        .transform(Transformation::rotate(percentage(f32::from(turn % 4) / 4.)));
    div()
        .size(px(size))
        .flex_none()
        .relative()
        .rounded_full()
        .border_2()
        .border_color(p.warn.opacity(0.25))
        .child(
            div()
                .absolute()
                .top(px(-3.))
                .left(px(-3.))
                .size(px(size + 2.))
                .child(arc),
        )
}

/// The blocked mark: a red disc with `!`.
pub(crate) fn blocked(p: &Pal, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(p.danger)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(size * 0.7))
        .font_weight(FontWeight::EXTRA_BOLD)
        .text_color(p.on_accent)
        .child("!")
}

/// The waiting-on-you mark: a filled ember dot in a soft ring.
pub(crate) fn needs_you(p: &Pal, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .border_2()
        .border_color(p.accent)
        .flex()
        .items_center()
        .justify_center()
        .child(div().size(px(size * 0.45)).rounded_full().bg(p.accent))
}

/// A row's lead mark for `lead`, the running ring turned `turn` quarters; none for the usual
/// state icon.
pub(crate) fn lead(p: &Pal, lead: Lead, turn: u8) -> Option<AnyElement> {
    let mark = match lead {
        Lead::Running => running(p, 13., turn).into_any_element(),
        Lead::Blocked => blocked(p, 15.).into_any_element(),
        Lead::NeedsYou => needs_you(p, 15.).into_any_element(),
        Lead::Usual => return None,
    };
    Some(
        div()
            .w(px(16.))
            .h(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .child(mark)
            .into_any_element(),
    )
}

/// A plan's bar, `height` high: a segment a step, or one plain fill.
pub(crate) fn bar(p: &Pal, bar: &Bar, height: f32) -> Div {
    match bar {
        Bar::Plain(fraction) => progress(p, *fraction, p.success).h(px(height)).mt(px(0.)),
        Bar::Segments(segments) => {
            div()
                .flex()
                .gap(px(2.))
                .w_full()
                .h(px(height))
                .children(segments.iter().map(|segment| {
                    let color = match segment {
                        Segment::Done => p.success,
                        Segment::Doing => p.warn,
                        Segment::Blocked => p.danger,
                        Segment::Todo => p.border,
                    };
                    div().flex_1().h_full().rounded(px(1.5)).bg(color)
                }))
        }
    }
}
