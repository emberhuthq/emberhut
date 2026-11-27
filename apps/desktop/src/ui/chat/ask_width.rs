//! The ask card's top line as the chat narrows, on GPUI's headless test platform: with more
//! asks and many in other rooms, the line stays inside the card and keeps the task's words; in a
//! narrow chat the other rooms' count moves to a line of its own.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, Pixels, TestAppContext, VisualTestContext, Window, px, size};
use serde_json::json;

use super::ask::{ASK_ELSEWHERE, ASK_EYEBROW, ASK_HIDE, ASK_TOP};
use super::tests::open_chat_over;
use crate::backend::fake::FakeBackend;
use crate::fixtures::{issue, room, state};
use emberhut_api::ids::RoomId;

/// A task in project 2 waiting on the user with a question, asked at `asked`.
fn ask(id: i64, asked: &str) -> serde_json::Value {
    let task = issue(
        id,
        "waiting",
        json!({"pid": 2, "title": "A task whose title is long enough to need the whole line",
            "decision": {"question": format!("Question {id}?"), "options": ["Yes", "No"],
                "asked": asked}}),
    );
    serde_json::to_value(task).unwrap()
}

/// Where the element with `selector` was drawn.
fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was drawn"))
}

#[gpui_kit::test]
fn the_top_line_stays_inside_a_narrow_card(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat_over(FakeBackend::new([]), cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    store.update(&mut cx, |store, cx| {
        store.take_state_for_tests(
            state(json!({
                "rooms": [room(1, json!({})), room(2, json!({})), room(3, json!({"needs": 52}))],
                "issues": [ask(10, "2026-11-25T08:00:00Z"), ask(11, "2026-11-25T09:00:00Z"),
                    ask(12, "2026-11-25T10:00:00Z")],
            })),
            cx,
        );
        store.switch_room(RoomId::new(2), cx);
    });
    let mut eyebrow_widths = Vec::new();
    for (width, inline) in [(900., true), (420., false), (360., false)] {
        cx.simulate_resize(size(px(width), px(800.)));
        cx.run_until_parked();
        cx.update(Window::render_frame);
        cx.update(Window::render_frame);
        let top = bounds(ASK_TOP, &mut cx);
        let elsewhere = bounds(ASK_ELSEWHERE, &mut cx);
        let hide = bounds(ASK_HIDE, &mut cx);
        eyebrow_widths.push(bounds(ASK_EYEBROW, &mut cx).size.width);
        assert!(
            top.right() <= px(width),
            "{width}: the line fits the chat: {top:?}"
        );
        assert!(
            hide.right() <= top.right(),
            "{width}: Hide stays on the line: {hide:?} in {top:?}"
        );
        if inline {
            assert!(
                elsewhere.top() >= top.top() && elsewhere.right() <= hide.left(),
                "{width}: the count sits on the line, before Hide: {elsewhere:?}, {hide:?}"
            );
        } else {
            assert!(
                elsewhere.top() >= top.bottom() && elsewhere.right() <= px(width),
                "{width}: the count has a line of its own: {elsewhere:?} under {top:?}"
            );
        }
    }
    assert!(
        eyebrow_widths
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() < px(1.)),
        "the task's words are whole at every width: {eyebrow_widths:?}"
    );
}
