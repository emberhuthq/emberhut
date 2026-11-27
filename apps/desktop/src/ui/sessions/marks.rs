//! The marks a session is drawn with: its live dot (it pulses while it runs, still under Reduce
//! motion), its state's mark as tasks mark theirs, a spinner, and the colour of each.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, ElementId, Hsla, Transformation, div,
    percentage, px,
};

use crate::store::sessions::words::Mark;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;
use crate::ui::widgets::dot;

/// A mark's colour.
#[must_use]
pub(crate) fn color(p: &Pal, mark: Mark) -> Hsla {
    match mark {
        Mark::Running | Mark::Done => p.success,
        Mark::Waiting => p.accent,
        Mark::Failed => p.danger,
        Mark::Stopped => p.warn,
    }
}

/// The live dot of a status: green and pulsing while it runs, else its mark's colour.
pub(crate) fn live_dot(p: &Pal, status: &str, id: impl Into<ElementId>, cx: &App) -> AnyElement {
    let mark = Mark::of(status);
    let fill = match mark {
        Mark::Done => p.faint,
        mark => color(p, mark),
    };
    let dot = dot(fill, 8.);
    if mark != Mark::Running || crate::motion::reduced(cx) {
        return dot.into_any_element();
    }
    dot.with_animation(
        id,
        Animation::new(Duration::from_millis(1600)).repeat(),
        |dot, t| dot.opacity(1. - 0.7 * (t * std::f32::consts::PI).sin()),
    )
    .into_any_element()
}

/// A session's state as tasks mark theirs: the live dot while it runs, a hand while it waits, a
/// cross, a stop, a tick.
pub(crate) fn state_mark(p: &Pal, status: &str, id: impl Into<ElementId>, cx: &App) -> AnyElement {
    let mark = Mark::of(status);
    let name = match mark {
        Mark::Running => {
            return div()
                .w(px(14.))
                .flex()
                .justify_center()
                .child(live_dot(p, status, id, cx))
                .into_any_element();
        }
        Mark::Waiting => IconName::Hand,
        Mark::Failed => IconName::CircleX,
        Mark::Stopped => IconName::CircleStop,
        Mark::Done => IconName::CircleCheck,
    };
    div()
        .w(px(14.))
        .flex()
        .justify_center()
        .child(glyph(name, color(p, mark), 12.5))
        .into_any_element()
}

/// A small spinner (a still circle under Reduce motion).
pub(crate) fn spinner(p: &Pal, id: impl Into<ElementId>, size: f32, cx: &App) -> AnyElement {
    let loader = glyph(IconName::LoaderCircle, p.accent, size);
    if crate::motion::reduced(cx) {
        return loader.into_any_element();
    }
    loader
        .with_animation(
            id,
            Animation::new(Duration::from_secs(1)).repeat(),
            |icon, t| icon.transform(Transformation::rotate(percentage(t))),
        )
        .into_any_element()
}
