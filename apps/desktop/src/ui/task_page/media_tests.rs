//! A run's thumbnails on a task's page, in a real window on GPUI's headless test platform: each
//! picture stays inside its 120 × 84 box whatever its shape (a phone video's tall frame, a
//! square side by side), and the next run starts below the thumbnails, not under them.

use std::io::Cursor;
use std::sync::Arc;

use emberhut_api::ids::TaskId;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Pixels, TestAppContext, VisualTestContext, Window, px, size,
};
use serde_json::{Value, json};

use super::work::{media_selector, picture_selector, run_selector};
use crate::backend::fake::FakeBackend;
use crate::fixtures::{state, task_page};
use crate::store::Store;
use crate::ui::palette::Pal;
use crate::ui::window::{self, MainWindow};

/// The task whose page is open.
const TASK: i64 = 7;

/// A blank PNG of `width` × `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::new(width, height)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("a PNG");
    bytes.into_inner()
}

/// Artifact `id`, a `kind` made at `at`, with a thumbnail.
fn artifact(id: i64, kind: &str, at: &str) -> Value {
    json!({"id": id, "kind": kind, "caption": null, "at": at, "name": format!("a{id}"),
           "url": format!("/api/artifacts/{id}"), "thumb": format!("/api/artifacts/{id}/thumb")})
}

/// Run `id`, from `started` to `ended`, done.
fn run(id: i64, started: &str, ended: &str) -> Value {
    json!({"id": id, "kind": "worker", "machine": "runner", "thread": format!("s-{id}"),
           "title": null, "asked": format!("Run {id}'s brief"), "answer": format!("Run {id}'s answer."),
           "status": "STATUS: done", "requested": null, "started": started, "ended": ended,
           "outcome": "completed"})
}

/// Where the element named `name` was drawn in the last frame.
fn bounds(cx: &mut VisualTestContext, name: String) -> Bounds<Pixels> {
    let leaked: &'static str = name.leak();
    cx.debug_bounds(leaked)
        .unwrap_or_else(|| panic!("{leaked} was drawn"))
}

/// Whether `inner` lies within `outer`, to half a point.
fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    let near = px(0.5);
    inner.left() >= outer.left() - near
        && inner.right() <= outer.right() + near
        && inner.top() >= outer.top() - near
        && inner.bottom() <= outer.bottom() + near
}

#[gpui_kit::test]
fn a_runs_thumbnails_stay_in_their_boxes_and_above_the_next_run(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
        window::bind_keys(cx);
    });
    // Run 2 made a phone video (9:16), a side-by-side (square) and a wide picture (16:9); run 1,
    // before it, a tall picture.
    let backend = FakeBackend::new([])
        .with_file("/api/artifacts/11/thumb", &png(360, 640))
        .with_file("/api/artifacts/12/thumb", &png(640, 640))
        .with_file("/api/artifacts/13/thumb", &png(640, 360))
        .with_file("/api/artifacts/14/thumb", &png(360, 640));
    backend.put_task(task_page(
        TASK,
        json!({
            "artifacts": [
                artifact(11, "video", "2026-01-01T02:59:00Z"),
                artifact(12, "video", "2026-01-01T02:59:10Z"),
                artifact(13, "image", "2026-01-01T02:59:20Z"),
                artifact(14, "image", "2026-01-01T00:59:00Z"),
            ],
            "runs": [
                run(2, "2026-01-01T02:00:00Z", "2026-01-01T03:00:00Z"),
                run(1, "2026-01-01T00:00:00Z", "2026-01-01T01:00:00Z"),
            ],
        }),
    ));
    let store = cx.new(|_| Store::for_tests(Arc::new(backend)));
    let (view, cx) = cx.add_window_view(|window, cx| {
        MainWindow::new(
            store.clone(),
            crate::ui::report::ReportSize::default(),
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(1440.), px(2400.)));
    store.update(cx, |store, cx| {
        store.take_state_for_tests(state(json!({})), cx);
    });
    view.update_in(cx, |main, window, cx| {
        main.open_task(TaskId::new(TASK), window, cx);
    });
    cx.run_until_parked();
    cx.update(Window::render_frame);

    let next_run = bounds(cx, run_selector(1));
    for artifact in [11, 12, 13] {
        let tile = bounds(cx, media_selector(2, artifact));
        let picture = bounds(cx, picture_selector(2, artifact));
        let close = |a: Pixels, b: f32| (a - px(b)).abs() <= px(1.);
        assert!(
            close(tile.size.width, 120.) && close(tile.size.height, 84.),
            "artifact {artifact}'s box is 120 × 84: {tile:?}"
        );
        assert!(
            inside(picture, tile),
            "artifact {artifact}'s picture stays in its box: {picture:?} in {tile:?}"
        );
        assert!(
            next_run.top() >= picture.bottom() && next_run.top() >= tile.bottom(),
            "run 1 starts below artifact {artifact}: {next_run:?} after {picture:?}"
        );
    }
    let last = bounds(cx, media_selector(1, 14));
    assert!(
        inside(bounds(cx, picture_selector(1, 14)), last),
        "run 1's tall picture stays in its box too"
    );
}
