//! The chat's messages as its width changes, on GPUI's headless test platform: an answer's
//! bubble takes the width the chat has now, as wide again once the chat is, without waiting for
//! anything else to redraw it.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Pixels, TestAppContext, VisualTestContext, Window, px, size};
use serde_json::json;

use super::tests::open_chat_over;
use crate::backend::fake::FakeBackend;
use crate::fixtures::{message_at, state};

/// An answer long enough to wrap in any of the widths below.
const LONG: &str = "The build finished and every check passed, the release notes are ready for a \
     final read before the upload goes out, and the backup ran overnight without a single error.";

/// The answer's bubble's width in the frame just drawn.
fn bubble_width(cx: &mut VisualTestContext) -> Pixels {
    cx.debug_bounds("bubble-1")
        .expect("the answer was drawn")
        .size
        .width
}

/// Gives the window `width` and lets it draw what that asks for.
fn resize(cx: &mut VisualTestContext, width: f32) -> Pixels {
    cx.simulate_resize(size(px(width), px(800.)));
    cx.run_until_parked();
    cx.update(Window::render_frame);
    cx.run_until_parked();
    bubble_width(cx)
}

#[gpui_kit::test]
fn an_answer_is_as_wide_again_once_the_chat_is(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat_over(FakeBackend::new([]), cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let answer = message_at(1, "2026-01-01T09:00:00Z", json!({"text": LONG}));
    store.update(&mut cx, |store, cx| {
        store.take_state_for_tests(state(json!({"chat": [answer]})), cx);
    });

    let wide = resize(&mut cx, 1000.);
    let narrow = resize(&mut cx, 500.);
    assert!(
        narrow < wide - px(100.),
        "narrower in a narrow chat: {narrow:?} < {wide:?}"
    );
    let again = resize(&mut cx, 1000.);
    assert!(
        (again - wide).abs() <= px(1.),
        "as wide again as at first: {again:?}, not {wide:?}"
    );
}
