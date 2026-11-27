//! The panel's inner edge: dragged, it resizes the panel; double-clicked, it returns it to the
//! default width. gpui-kit's resize handle gives the band its resize cursor and its line.
//!
//! The band is a column of its own beside the panel's content, not laid over it: the web view's
//! native view covers whatever GPUI draws in its bounds, and over the chat the band would cover
//! its scroll bar.

use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::prelude::*;
use gpui_kit::{
    Axis, Div, DragMoveEvent, EmptyView, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Window,
    div, px,
};

use super::super::window::MainWindow;
use super::space;

/// The band's width: gpui-kit's 1 px line and the 4 px of padding on its inner side.
pub(super) const BAND: Pixels = px(5.);

/// The edge's element id, and its selector for the tests (a no-op in a normal build).
pub(super) const EDGE: &str = "report-edge";

/// What a drag of the edge carries: GPUI tells drags apart by the type of their payload.
#[derive(Clone, Copy, Debug)]
struct EdgeDrag;

/// The edge, on the panel's left.
pub(super) fn edge(cx: &mut Context<'_, MainWindow>) -> Div {
    let handle = resize_handle::<EdgeDrag, EmptyView>(EDGE, Axis::Horizontal)
        .inside(HandleEdge::Trailing)
        // Nothing follows the pointer: the panel's edge is what moves.
        .on_drag(EdgeDrag, |_, _, _, cx| cx.new(|_| EmptyView))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                let space = space(this, window, cx);
                this.report_size.press(event.position.x, space);
            }),
        )
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<EdgeDrag>, window, cx| {
                let space = space(this, window, cx);
                if this.report_size.drag_to(event.event.position.x, space) {
                    cx.notify();
                }
            }),
        )
        // Let go on the band or anywhere else.
        .on_mouse_up(MouseButton::Left, cx.listener(release))
        .on_mouse_up_out(MouseButton::Left, cx.listener(release))
        .on_double_click(cx.listener(|this, _, _, cx| {
            this.report_size.reset(cx);
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

/// The pointer let go of the edge: a width the drag changed is kept.
fn release(
    this: &mut MainWindow,
    _: &MouseUpEvent,
    window: &mut Window,
    cx: &mut Context<'_, MainWindow>,
) {
    let space = space(this, window, cx);
    this.report_size.release(space, cx);
}
