//! The circle that ticks the user's own to-do done, on a task's row and on a goal's list: a
//! click fills it with a green check that bounces once (held still when the system asks for
//! reduced motion, as every GPUI animation is), and the task leaves the list a beat later
//! ([`Store::tick_done`]).

use std::time::Duration;

use emberhut_api::ids::TaskId;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Animation, AnimationExt as _, Div, ElementId, Entity, Stateful, div, px};

use super::marks::glyph;
use crate::store::Store;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;

/// How long the check's bounce takes.
const BOUNCE: Duration = Duration::from_millis(320);

/// How much bigger the check grows at the height of its bounce.
const BOUNCE_GROWTH: f32 = 0.3;

/// The check's size at `delta` (0 to 1) through its bounce, for a circle `size` pixels wide: it
/// grows and settles back.
#[must_use]
fn bounced(size: f32, delta: f32) -> f32 {
    size * (1.0 + BOUNCE_GROWTH * (std::f32::consts::PI * delta.clamp(0.0, 1.0)).sin())
}

/// The tick for the task `id` called `title`, `size` pixels wide, in a box of that size so the
/// row stays put while it bounces.
pub(crate) fn tick(
    store: &Entity<Store>,
    p: &Pal,
    id: TaskId,
    title: &str,
    size: f32,
    ticked: bool,
) -> Stateful<Div> {
    let mark = if ticked {
        glyph(IconName::CircleCheck, p.success, size)
            .with_animation(
                ElementId::Name(format!("tick-bounce-{id}").into()),
                Animation::new(BOUNCE),
                move |icon, delta| icon.size(px(bounced(size, delta))),
            )
            .into_any_element()
    } else {
        glyph(IconName::Circle, p.muted, size).into_any_element()
    };
    let store = store.clone();
    div()
        .id(ElementId::Name(format!("tick-{id}").into()))
        .button_named(format!("Mark {title} done"))
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .child(mark)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            store.update(cx, |store, cx| store.tick_done(id, cx));
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_check_grows_and_settles_back() {
        assert!((bounced(16.0, 0.0) - 16.0).abs() < 1e-4);
        assert!((bounced(16.0, 0.5) - 16.0 * (1.0 + BOUNCE_GROWTH)).abs() < 1e-4);
        assert!(
            (bounced(16.0, 1.0) - 16.0).abs() < 1e-3,
            "its end, held still"
        );
    }
}
