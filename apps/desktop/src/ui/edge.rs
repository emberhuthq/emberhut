//! The left edge of the column beside the chat: dragged, it makes the column wider or narrower
//! (the chat, and a report open beside both, keep their narrowest widths, as
//! [`task_page::column_width`] rules); double-clicked, it gives the column its standard width
//! again. The width the user gives is kept across launches ([`Prefs::column_width`]).
//!
//! [`task_page::column_width`]: super::task_page::column_width
//! [`Prefs::column_width`]: crate::prefs::Prefs::column_width

use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::prelude::*;
use gpui_kit::{
    Axis, Div, DragMoveEvent, EmptyView, MouseButton, MouseUpEvent, Pixels, Window, div, px,
};

use super::window::MainWindow;
use crate::prefs::Kept;

/// The band's width: gpui-kit's 1 px line and the 4 px of padding on its inner side.
pub(crate) const BAND: Pixels = px(5.);

/// The edge's element id, and its selector for the tests (a no-op in a normal build).
pub(crate) const EDGE: &str = "column-edge";

/// What a drag of the edge carries: GPUI tells drags apart by the type of their payload.
#[derive(Clone, Copy, Debug)]
struct EdgeDrag;

/// The width a drag gives the column, `width` wide now with its edge's left side at `left`, the
/// pointer at `x`: its right side stays, its left edge follows the pointer.
#[must_use]
pub(crate) fn dragged(left: Pixels, width: Pixels, x: Pixels) -> Pixels {
    left + width - x
}

/// The edge, on the column's left (within its width).
pub(crate) fn edge(cx: &mut Context<'_, MainWindow>) -> Div {
    let handle = resize_handle::<EdgeDrag, EmptyView>(EDGE, Axis::Horizontal)
        .inside(HandleEdge::Trailing)
        // Nothing follows the pointer: the column's edge is what moves.
        .on_drag(EdgeDrag, |_, _, _, cx| cx.new(|_| EmptyView))
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<EdgeDrag>, window, cx| {
                let Some(width) = this.column_width(window, cx) else {
                    return;
                };
                let asked = dragged(event.bounds.left(), width, event.event.position.x);
                let Some(given) = this.column_width_for(window, Some(asked), cx) else {
                    return;
                };
                if given != width {
                    Kept::note_column_width(cx, Some(f32::from(given)));
                    cx.notify();
                }
            }),
        )
        // Let go on the band or anywhere else: the width is written once.
        .on_mouse_up(MouseButton::Left, cx.listener(release))
        .on_mouse_up_out(MouseButton::Left, cx.listener(release))
        .on_double_click(cx.listener(|_, _, _, cx| {
            Kept::note_column_width(cx, None);
            Kept::save_column_width(cx);
            cx.notify();
        }));
    div()
        .debug_selector(|| EDGE.to_owned())
        .relative()
        .flex_none()
        .w(BAND)
        .h_full()
        .child(handle)
}

/// The pointer let go (of the edge, or anywhere after a drag of it): a width it was dragged to
/// is written.
fn release(_: &mut MainWindow, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<'_, MainWindow>) {
    Kept::save_column_width(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_left_makes_the_column_wider() {
        let at = |left: f32, width: f32, x: f32| f32::from(dragged(px(left), px(width), px(x)));
        assert!((at(900., 500., 800.) - 600.).abs() < f32::EPSILON);
        assert!((at(900., 500., 1000.) - 400.).abs() < f32::EPSILON);
        assert!((at(900., 500., 900.) - 500.).abs() < f32::EPSILON, "still");
    }
}
