//! The bar over the chat, on GPUI's headless test platform: as the chat narrows, with a long
//! room name and asks waiting, everything on it stays inside the chat; Main's is named as the
//! user named the ember.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, Pixels, TestAppContext, VisualTestContext, Window, px, size};
use serde_json::json;

use super::header::{HEADER, ROOM_NEEDS, TITLE};
use super::tests::open_chat_over;
use crate::backend::fake::FakeBackend;
use crate::fixtures::{message_at, room, state};
use emberhut_api::ids::RoomId;

/// Where the element with `selector` was drawn.
fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was drawn"))
}

#[gpui_kit::test]
fn the_bar_stays_inside_a_narrow_chat(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat_over(FakeBackend::new([]), cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let name = "A project whose name is long enough to need the whole bar and more";
    store.update(&mut cx, |store, cx| {
        store.take_state_for_tests(
            state(json!({
                "rooms": [room(1, json!({})), room(2, json!({"name": name, "needs": 3}))],
                "chat": [message_at(1, "2026-01-01T09:00:00Z", json!({"text": "Hello"}))],
            })),
            cx,
        );
        store.switch_room(RoomId::new(2), cx);
    });
    for width in [900., 420., 340.] {
        cx.simulate_resize(size(px(width), px(800.)));
        cx.run_until_parked();
        cx.update(Window::render_frame);
        cx.update(Window::render_frame);
        let bar = bounds(HEADER, &mut cx);
        let needs = bounds(ROOM_NEEDS, &mut cx);
        assert!(bar.right() <= px(width), "{width}: the bar fits: {bar:?}");
        assert!(
            needs.right() <= bar.right() && needs.size.width > px(0.),
            "{width}: the needs button is whole, inside the bar: {needs:?} in {bar:?}"
        );
    }
}

#[gpui_kit::test]
fn main_s_bar_names_the_ember_as_the_user_named_it(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat_over(FakeBackend::new([]), cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let title = |named: Option<&str>, cx: &mut VisualTestContext| {
        let ember = named.map(|name| json!({ "name": name, "custom": true, "at": null }));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(
                state(json!({ "rooms": [room(1, json!({}))], "emberName": ember })),
                cx,
            );
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.find(TITLE).label().map(str::to_owned)
        })
    };
    assert_eq!(
        title(None, &mut cx).as_deref(),
        Some("Your ember"),
        "the default name"
    );
    assert_eq!(title(Some("Pip"), &mut cx).as_deref(), Some("Pip"));
}
