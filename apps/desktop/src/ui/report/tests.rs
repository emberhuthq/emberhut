//! The panel's web view over the store's open report, on GPUI's headless test platform with the
//! fake backend: made when a fetched report shows, kept while it does, dropped when the panel
//! closes, shows another report, or another screen takes the app's place. A counting stand-in takes the web view's place (the real one
//! needs a window).

use std::convert::Infallible;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Entity, TestAppContext};

use super::slot::Slot;
use super::slot::tests::{Probe, Probes};
use super::{open_report, sync, web_view_off};
use crate::backend::fake::FakeBackend;
use crate::store::{Fetched, OpenReport, Store};

const A: &str = "/files/a.html";

const B: &str = "/files/b.html";

fn report(path: &str) -> OpenReport {
    OpenReport {
        name: path.trim_start_matches("/files/").to_owned(),
        path: path.to_owned(),
    }
}

/// Runs one frame of the panel's logic over the store's report, as the panel does, and says
/// whether the stand-in web view is up.
fn frame(
    store: &Entity<Store>,
    slot: &mut Slot<Probe, Infallible>,
    probes: &Probes,
    cx: &mut TestAppContext,
) -> bool {
    cx.read(|cx| {
        let open = open_report(store.read(cx));
        sync(slot, open.as_ref(), |_html| Ok(probes.make())).is_some()
    })
}

/// A store over `backend` with a first state, so the app (and its report panel) shows.
fn app_store(backend: FakeBackend, cx: &mut TestAppContext) -> Entity<Store> {
    let store = cx.new(|_| Store::for_tests(Arc::new(backend)));
    store.update(cx, |store, cx| {
        store.take_state_for_tests(crate::fixtures::state(serde_json::json!({})), cx);
    });
    store
}

fn show(store: &Entity<Store>, path: Option<&str>, cx: &mut TestAppContext) {
    store.update(cx, |store, cx| store.show_report(path.map(report), cx));
}

#[test]
fn only_off_turns_the_web_view_off() {
    use std::ffi::OsStr;

    assert!(web_view_off(Some(OsStr::new("off"))));
    assert!(web_view_off(Some(OsStr::new("OFF"))));
    for on in [None, Some(""), Some("on"), Some("1"), Some("offline")] {
        assert!(!web_view_off(on.map(OsStr::new)), "{on:?}");
    }
}

#[gpui_kit::test]
fn one_web_view_follows_the_open_report(cx: &mut TestAppContext) {
    let backend = FakeBackend::new([])
        .with_file(
            A,
            b"<!doctype html><title>Sales &amp; costs</title><p>a</p>",
        )
        .with_file(B, b"<h1>Second</h1>");
    let store = app_store(backend, cx);
    let probes = Probes::default();
    let mut slot = Slot::default();

    assert!(!frame(&store, &mut slot, &probes, cx), "no report open");

    // Opened: nothing while it loads, then one web view, kept frame after frame.
    show(&store, Some(A), cx);
    assert!(!frame(&store, &mut slot, &probes, cx), "loading");
    cx.run_until_parked();
    for _ in 0..3 {
        assert!(frame(&store, &mut slot, &probes, cx));
    }
    assert_eq!((probes.made(), probes.live()), (1, 1));
    let title = cx.read(|cx| match store.read(cx).report(A) {
        Fetched::Ready(page) => page.title,
        other => panic!("not fetched: {other:?}"),
    });
    assert_eq!(title.as_deref(), Some("Sales & costs"));

    // Another report: the first view goes at once, the next comes with its page.
    show(&store, Some(B), cx);
    assert!(!frame(&store, &mut slot, &probes, cx));
    assert_eq!(probes.live(), 0);
    cx.run_until_parked();
    assert!(frame(&store, &mut slot, &probes, cx));
    assert_eq!((probes.made(), probes.live()), (2, 1));

    // Closed: no view kept.
    show(&store, None, cx);
    assert!(!frame(&store, &mut slot, &probes, cx));
    assert_eq!((probes.live(), slot.key()), (0, None));

    // Opened again (already fetched): a new view straight away.
    show(&store, Some(A), cx);
    assert!(frame(&store, &mut slot, &probes, cx));
    assert_eq!((probes.made(), probes.live()), (3, 1));
}

#[gpui_kit::test]
fn the_web_view_goes_while_another_screen_takes_the_apps_place(cx: &mut TestAppContext) {
    let backend = FakeBackend::new([]).with_file(A, b"<p>a</p>");
    let store = app_store(backend, cx);
    let probes = Probes::default();
    let mut slot = Slot::default();
    show(&store, Some(A), cx);
    cx.run_until_parked();
    assert!(frame(&store, &mut slot, &probes, cx));
    store.update(cx, Store::end_session);
    assert!(!frame(&store, &mut slot, &probes, cx), "beside sign-in");
    assert_eq!(probes.live(), 0);
}

#[gpui_kit::test]
fn a_report_that_fails_to_fetch_has_no_web_view(cx: &mut TestAppContext) {
    let backend = FakeBackend::new([]).with_file(A, b"<p>a</p>");
    let store = app_store(backend, cx);
    let probes = Probes::default();
    let mut slot = Slot::default();

    show(&store, Some(A), cx);
    cx.run_until_parked();
    assert!(frame(&store, &mut slot, &probes, cx));

    show(&store, Some("/files/gone.html"), cx);
    cx.run_until_parked();
    let fetched = cx.read(|cx| store.read(cx).report("/files/gone.html"));
    assert!(matches!(fetched, Fetched::Failed(_)), "{fetched:?}");
    assert!(!frame(&store, &mut slot, &probes, cx));
    assert_eq!(probes.live(), 0);
}

/// The real panel in a window, where there is no web view (on macOS the test window has no
/// native view to host one): it falls back to rich text, keeping that for the report, and keeps
/// nothing once Esc closes it.
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn without_a_web_view_the_panel_falls_back_and_closes(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;

    use super::{RICH_TEXT, ReportSize};
    use crate::ui::palette::Pal;
    use crate::ui::window::{CloseReport, MainWindow};

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = FakeBackend::new([]).with_file(A, b"<h1>Sales</h1><p>a</p>");
    let store = app_store(backend, cx);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MainWindow::new(store.clone(), ReportSize::default(), window, cx)
    });
    cx.update(TestWindowExt::render_frame);
    assert_eq!(cx.debug_bounds(RICH_TEXT), None, "no report open yet");

    store.update(cx, |store, cx| store.show_report(Some(report(A)), cx));
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    assert!(
        cx.debug_bounds(RICH_TEXT).is_some(),
        "the report as rich text"
    );
    // Here the slot can only hold the failure, kept for the report.
    view.read_with(cx, |view, _| {
        assert_eq!(view.report_body.slot.key(), Some(A));
    });

    cx.dispatch_action(CloseReport);
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    assert_eq!(cx.read(|cx| store.read(cx).open_report().cloned()), None);
    assert_eq!(cx.debug_bounds(RICH_TEXT), None);
    view.read_with(cx, |view, _| {
        assert_eq!(view.report_body.slot.key(), None, "nothing kept");
    });
}

/// A window with a report open that locks holds no web view while the lock's cover shows (its
/// native view would be drawn over the cover), and makes it again once unlocked.
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn a_locked_window_holds_no_web_view(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;

    use super::ReportSize;
    use crate::lock::Lock;
    use crate::ui::palette::Pal;
    use crate::ui::window::MainWindow;

    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = FakeBackend::new([]).with_file(A, b"<h1>Sales</h1><p>a</p>");
    let store = app_store(backend, cx);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MainWindow::new(store.clone(), ReportSize::default(), window, cx)
    });
    store.update(cx, |store, cx| store.show_report(Some(report(A)), cx));
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    view.read_with(cx, |view, _| {
        assert_eq!(view.report_body.slot.key(), Some(A), "the report shows");
    });

    cx.update(|_, cx| cx.set_global(Lock::at_launch(true, false)));
    cx.update(TestWindowExt::render_frame);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.report_body.slot.key(),
            None,
            "nothing kept under the cover"
        );
    });

    cx.update(|_, cx| cx.set_global(Lock::at_launch(true, true)));
    cx.update(TestWindowExt::render_frame);
    view.read_with(cx, |view, _| {
        assert_eq!(view.report_body.slot.key(), Some(A), "back once unlocked");
    });
}
