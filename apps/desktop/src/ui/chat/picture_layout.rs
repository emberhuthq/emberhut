//! Pictures in the chat's list, on GPUI's headless test platform: each message holds its
//! pictures whole, its chips come after them and the next message after both, while a picture
//! loads and once it has come, in view or out of it; a click on one opens it.

use std::cell::RefCell;
use std::io::Cursor;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext, Window, px, size};
use serde_json::{Value, json};

use super::tests::open_chat_over;
use crate::backend::fake::{Call, FakeBackend};
use crate::fixtures::{message_at, state};
use crate::store::files::RemoteFile;

/// A phone screenshot, 390 × 844 pixels: 166 × 360 shown.
const TALL: &str = "/uploads/screen@2x.png";

/// A banner, 1600 × 400 pixels: 320 × 80 shown.
const WIDE: &str = "/uploads/banner.png";

/// A picture the server never sends: it stays as its placeholder.
const MISSING: &str = "/uploads/missing.png";

/// A blank PNG of `width` × `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::new(width, height)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("a PNG");
    bytes.into_inner()
}

/// The assistant's message `id` with a picture at `url` and two chips under it.
fn with_picture(id: i64, url: &str) -> Value {
    serde_json::to_value(message_at(
        id,
        "2026-01-01T09:00:00Z",
        json!({
            "text": format!("Message {id}, with a picture."),
            "files": [{"name": url.trim_start_matches("/uploads/"), "url": url, "kind": "image"}],
            "refs": [
                {"kind": "task", "label": format!("T-{id}"), "title": "Example task"},
                {"kind": "pr", "label": "PR #7", "title": "example/repo", "url": "https://example.com/pr/7"},
            ],
        }),
    ))
    .expect("a message")
}

/// The chat: a tall picture, a wide one, one that never comes, then the user's reply.
fn chat() -> Value {
    let reply = message_at(
        4,
        "2026-01-01T09:00:00Z",
        json!({"mine": true, "text": "Thanks"}),
    );
    json!([
        with_picture(1, TALL),
        with_picture(2, WIDE),
        with_picture(3, MISSING),
        serde_json::to_value(reply).expect("a message"),
    ])
}

/// The bounds the last frame drew the element named `name` at.
fn bounds(cx: &mut VisualTestContext, name: String) -> Bounds<Pixels> {
    let leaked: &'static str = name.leak();
    cx.debug_bounds(leaked)
        .unwrap_or_else(|| panic!("{leaked} was drawn"))
}

/// Checks message `id`'s picture, chips and the message `next` after it, and gives back the
/// picture's bounds and the next message's.
fn check(cx: &mut VisualTestContext, id: i64, next: i64) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let message = bounds(cx, format!("message-{id}"));
    let picture = bounds(cx, format!("picture-{id}-0"));
    let chips = bounds(cx, format!("chips-{id}"));
    let after = bounds(cx, format!("message-{next}"));
    assert!(
        message.left() <= picture.left()
            && picture.right() <= message.right()
            && message.top() <= picture.top()
            && picture.bottom() <= message.bottom(),
        "message {id} holds its picture: {picture:?} in {message:?}"
    );
    assert!(
        chips.top() >= picture.bottom(),
        "message {id}'s chips come after its picture: {chips:?} after {picture:?}"
    );
    assert!(
        after.top() >= chips.bottom() && after.top() >= picture.bottom(),
        "message {next} comes after message {id}: {after:?} after {chips:?} and {picture:?}"
    );
    (picture, after)
}

fn assert_size(picture: Bounds<Pixels>, width: f32, height: f32) {
    let close = |a: Pixels, b: f32| (a - px(b)).abs() <= px(1.);
    assert!(
        close(picture.size.width, width) && close(picture.size.height, height),
        "{picture:?} is {width} × {height}"
    );
}

#[gpui_kit::test]
fn pictures_take_their_room_in_the_list_before_and_after_they_come(cx: &mut TestAppContext) {
    let backend = FakeBackend::new([])
        .with_file(TALL, &png(390, 844))
        .with_file(WIDE, &png(1600, 400));
    let (_backend, store, window) = open_chat_over(backend, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.simulate_resize(size(px(1000.), px(1800.)));
    store.update(&mut cx, |store, cx| {
        store.take_state_for_tests(state(json!({"chat": chat()})), cx);
    });

    // First drawn while every picture is on its way: each a placeholder.
    cx.update(Window::render_frame);
    let first = bounds(&mut cx, "message-1".to_owned());
    let (tall, second) = check(&mut cx, 1, 2);
    let (wide, third) = check(&mut cx, 2, 3);
    let (missing, _) = check(&mut cx, 3, 4);
    for placeholder in [tall, wide, missing] {
        assert_size(placeholder, 240., 140.);
    }

    // The pictures come: each at its own shape, within 320 × 360, and the rows below it move
    // down from it (the list keeps its last row at the bottom, so the rows above move up).
    cx.run_until_parked();
    cx.update(Window::render_frame);
    let first_now = bounds(&mut cx, "message-1".to_owned());
    let (tall, second_now) = check(&mut cx, 1, 2);
    let (wide, third_now) = check(&mut cx, 2, 3);
    let (missing, _) = check(&mut cx, 3, 4);
    let tall_height = 360.;
    assert_size(tall, 390. * tall_height / 844., tall_height);
    assert_size(wide, 320., 80.);
    assert_size(missing, 240., 140.);
    let below = |row: Bounds<Pixels>, first: Bounds<Pixels>| (row.top() - first.top()).as_f32();
    let pushed = below(second_now, first_now) - below(second, first);
    assert!(
        (pushed - (tall_height - 140.)).abs() <= 1.,
        "the tall picture pushed the next message down by its growth: {pushed}"
    );
    let pushed = below(third_now, first_now) - below(third, first);
    assert!(
        (pushed - (tall_height - 140. + 80. - 140.)).abs() <= 1.,
        "and the one after by both pictures' change: {pushed}"
    );
}

/// How much the list's scroll range grows when `pictures` messages, each showing the same tall
/// picture, are just above the view (where the list measures rows but keeps their heights)
/// and the picture comes.
fn growth_out_of_view(pictures: i64, cx: &mut TestAppContext) -> f32 {
    let backend = FakeBackend::new([]).with_file(TALL, &png(390, 844));
    let (_backend, store, window) = open_chat_over(backend, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.simulate_resize(size(px(1000.), px(700.)));
    let mut chat: Vec<Value> = (1..=pictures).map(|id| with_picture(id, TALL)).collect();
    let last = pictures + 15;
    chat.extend((pictures + 1..=last).map(|id| {
        let reply = message_at(
            id,
            "2026-01-01T09:00:00Z",
            json!({"mine": true, "text": format!("Reply {id}")}),
        );
        serde_json::to_value(reply).expect("a message")
    }));
    store.update(&mut cx, |store, cx| {
        store.take_state_for_tests(state(json!({"chat": chat})), cx);
    });
    let scroll_range = |cx: &mut VisualTestContext| {
        window
            .update(cx, |view, _, _| view.list.max_offset_for_scrollbar().y)
            .expect("the chat")
    };
    let out_of_view = |cx: &mut VisualTestContext| {
        (1..=pictures).all(|id| {
            let name: &'static str = format!("message-{id}").leak();
            cx.debug_bounds(name).is_none()
        })
    };

    cx.update(Window::render_frame);
    assert!(out_of_view(&mut cx), "the pictures' rows are out of view");
    let in_view = bounds(&mut cx, format!("message-{last}"));
    let before = scroll_range(&mut cx);

    cx.run_until_parked();
    cx.update(Window::render_frame);
    assert!(out_of_view(&mut cx), "and still are");
    assert_eq!(
        bounds(&mut cx, format!("message-{last}")),
        in_view,
        "what is in view stays where it was"
    );
    (scroll_range(&mut cx) - before).as_f32()
}

#[gpui_kit::test]
fn a_picture_that_comes_out_of_view_is_measured_again(cx: &mut TestAppContext) {
    let grown = growth_out_of_view(1, cx);
    assert!(
        (grown - (360. - 140.)).abs() <= 1.,
        "the list holds the picture at its size: grew by {grown}"
    );
}

#[gpui_kit::test]
fn a_picture_shown_twice_is_measured_again_in_both_rows(cx: &mut TestAppContext) {
    let grown = growth_out_of_view(2, cx);
    assert!(
        (grown - 2. * (360. - 140.)).abs() <= 1.,
        "both rows hold the picture at its size: grew by {grown}"
    );
}

#[gpui_kit::test]
fn a_click_on_a_picture_shows_the_whole_file_in_the_viewer(cx: &mut TestAppContext) {
    let picture = png(390, 844);
    let backend = FakeBackend::new([]).with_file(TALL, &picture);
    backend.push_download(Ok(picture));
    let (backend, store, window) = open_chat_over(backend, cx);
    let root = tempfile::tempdir().expect("a folder");
    let opened = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.simulate_resize(size(px(1000.), px(1200.)));
    store.update(&mut cx, |store, cx| {
        store.keep_files_in(root.path(), &opened);
        let chat = json!([with_picture(1, TALL)]);
        store.take_state_for_tests(state(json!({"chat": chat})), cx);
    });
    cx.run_until_parked();
    cx.update(Window::render_frame);

    let shown = bounds(&mut cx, "picture-1-0".to_owned());
    assert_size(shown, 390. * 360. / 844., 360.);
    cx.simulate_click(shown.center(), Modifiers::none());
    cx.run_until_parked();

    let downloads: Vec<Call> = backend
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Download(_)))
        .collect();
    assert_eq!(downloads, [Call::Download(TALL.to_owned())]);
    assert!(
        opened.borrow().is_empty(),
        "shown in the viewer, not handed to another app"
    );
    let file = RemoteFile::new(TALL, TALL.trim_start_matches("/uploads/"));
    let kept = store.read_with(&cx, |store, _| store.kept_file(&file));
    assert_eq!(
        std::fs::read(kept.expect("kept")).expect("readable"),
        png(390, 844),
        "the whole file, fetched for the viewer"
    );
}
