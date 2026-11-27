//! The panel's size in a real window on GPUI's headless test platform, with the fake backend:
//! the edge dragged and double-clicked, the width kept for the next launch, the panel expanded
//! and restored by its button, its shortcut and Esc, the page changes that restore it, where the
//! keyboard focus goes, the answer toast while it is expanded, and the room the task column
//! leaves the page and the panel.
//!
//! Most reports shown here fail to fetch, so no platform makes a web view for them: the layout
//! is the same with one. The rich text fallback has a test of its own.

use std::sync::Arc;

use emberhut_api::ids::TaskId;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point, px, size,
};
use serde_json::json;

use super::edge::EDGE;
use super::kept::KeptWidth;
use super::width::{PanelWidth, Space};
use super::{BODY, EXPAND, PANEL, ReportSize};
use crate::backend::fake::FakeBackend;
use crate::config::AppFiles;
use crate::fixtures::{room, state};
use crate::store::{OpenReport, Page, Store};
use crate::ui::palette::Pal;
use crate::ui::toast::TOAST;
use crate::ui::window::{self, CHAT, COLUMN, MainWindow, PAGE};

/// The shortcut that expands the report and brings it back.
const TOGGLE: &str = if cfg!(target_os = "macos") {
    "cmd-shift-enter"
} else {
    "ctrl-shift-enter"
};

/// `key` with the platform's primary modifier: ⌘ on the Mac, Ctrl elsewhere.
fn primary(key: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-{key}")
}

/// The report whose fetch fails.
fn gone() -> OpenReport {
    OpenReport {
        name: "gone.html".to_owned(),
        path: "/files/gone.html".to_owned(),
    }
}

/// A window 1260 px wide (1000 px of it beside the sidebar) over a fresh store on `backend`
/// with a first state (so the app is open, past its connecting screen), its panel at
/// `report_size`.
fn open_window_over(
    backend: FakeBackend,
    report_size: ReportSize,
    cx: &mut TestAppContext,
) -> (Entity<Store>, Entity<MainWindow>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
        window::bind_keys(cx);
    });
    let store = cx.new(|_| Store::for_tests(Arc::new(backend)));
    let (view, cx) =
        cx.add_window_view(|window, cx| MainWindow::new(store.clone(), report_size, window, cx));
    store.update(cx, |store, cx| {
        store.take_state_for_tests(state(json!({})), cx);
    });
    cx.simulate_resize(size(px(1260.), px(800.)));
    (store, view, cx)
}

/// [`open_window_over`] a backend with no files.
fn open_window(
    report_size: ReportSize,
    cx: &mut TestAppContext,
) -> (Entity<Store>, Entity<MainWindow>, &mut VisualTestContext) {
    open_window_over(FakeBackend::new([]), report_size, cx)
}

/// Opens `report` and lets its fetch end.
fn show(report: OpenReport, store: &Entity<Store>, cx: &mut VisualTestContext) {
    store.update(cx, |store, cx| store.show_report(Some(report), cx));
    cx.run_until_parked();
}

/// Opens the report whose fetch fails.
fn show_report(store: &Entity<Store>, cx: &mut VisualTestContext) {
    show(gone(), store, cx);
}

/// Draws a frame and says where the element with `selector` is, if it is drawn.
fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.update(TestWindowExt::render_frame);
    cx.debug_bounds(selector)
}

/// The width of the element with `selector`, if it is drawn.
fn width_of(selector: &'static str, cx: &mut VisualTestContext) -> Option<Pixels> {
    bounds(selector, cx).map(|bounds| bounds.size.width)
}

/// The panel's width, if it shows.
fn panel_width(cx: &mut VisualTestContext) -> Option<Pixels> {
    width_of(PANEL, cx)
}

/// The middle of the panel's edge.
fn edge(cx: &mut VisualTestContext) -> Point<Pixels> {
    bounds(EDGE, cx).expect("the edge shows").center()
}

/// Drags the panel's edge from `from` to `to`, with a first small move that starts the drag.
fn drag(from: Point<Pixels>, to: Point<Pixels>, cx: &mut VisualTestContext) {
    let none = Modifiers::none();
    cx.simulate_mouse_down(from, MouseButton::Left, none);
    cx.simulate_mouse_move(from - point(px(4.), px(0.)), MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    cx.run_until_parked();
}

/// A double click at `at`.
fn double_click(at: Point<Pixels>, cx: &mut VisualTestContext) {
    for click_count in 1..=2 {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    cx.run_until_parked();
}

/// Clicks the element with `selector`.
fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let at = bounds(selector, cx).expect("it shows").center();
    cx.simulate_click(at, Modifiers::none());
}

/// Clicks the element whose id is `id`, in a frame drawn first.
fn click_id(id: &'static str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    });
}

/// The width kept in `files`.
fn kept(files: &AppFiles) -> PanelWidth {
    KeptWidth::load(files.clone()).width()
}

fn is_expanded(view: &Entity<MainWindow>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.report_size.is_expanded())
}

fn composer_focused(view: &Entity<MainWindow>, cx: &mut VisualTestContext) -> bool {
    cx.update(TestWindowExt::render_frame);
    cx.update(|window, cx| view.read(cx).chat.read(cx).composer_focused(window, cx))
}

fn focus_composer(view: &Entity<MainWindow>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let chat = view.read(cx).chat.clone();
        chat.update(cx, |chat, cx| chat.focus_composer(window, cx));
    });
    assert!(composer_focused(view, cx));
}

/// Where `view` is: the page its column shows, else the chat's room.
fn page(view: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Page {
    cx.read(|cx| {
        let main = view.read(cx);
        main.column
            .read(cx)
            .current()
            .unwrap_or(Page::Room(main.store.read(cx).room()))
    })
}

/// The page and the panel share the content area: nothing else in the row takes room.
fn assert_they_share_the_content_area(cx: &mut VisualTestContext) {
    let (Some(page), Some(panel)) = (width_of(PAGE, cx), panel_width(cx)) else {
        panic!("the page and the panel both show");
    };
    assert_eq!(page + panel, px(1000.));
}

#[gpui_kit::test]
fn the_edge_resizes_the_panel_and_the_width_is_kept(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let (store, _, cx) = open_window(ReportSize::load(Some(&files)), cx);
    show_report(&store, cx);
    assert_eq!(panel_width(cx), Some(px(580.)), "46% of 1260");
    assert_they_share_the_content_area(cx);

    // 50 px to the left: 50 px wider, kept as a share of the window.
    let from = edge(cx);
    drag(from, from - point(px(50.), px(0.)), cx);
    assert_eq!(panel_width(cx), Some(px(630.)));
    assert_they_share_the_content_area(cx);
    assert!((kept(&files).share() - 0.5).abs() < 1e-6);
    assert_eq!(
        ReportSize::load(Some(&files)).pixels(Space::new(px(1260.), px(1000.))),
        px(630.),
        "the next launch opens at it"
    );

    // The window doubles: so does the panel.
    cx.simulate_resize(size(px(2520.), px(800.)));
    assert_eq!(panel_width(cx), Some(px(1260.)));
    cx.simulate_resize(size(px(1260.), px(800.)));

    // Far to the left: the chat keeps 360 px.
    let from = edge(cx);
    drag(from, point(px(0.), from.y), cx);
    assert_eq!(panel_width(cx), Some(px(640.)));
    assert_eq!(width_of(PAGE, cx), Some(px(360.)));

    // A double click: back to the default, kept too.
    double_click(edge(cx), cx);
    assert_eq!(panel_width(cx), Some(px(580.)));
    assert_eq!(kept(&files), PanelWidth::DEFAULT);
}

#[gpui_kit::test]
fn a_drag_that_shows_nothing_new_keeps_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let (store, _, cx) = open_window(ReportSize::load(Some(&files)), cx);
    // 640 px beside the sidebar: too narrow for both, the panel is 380 px whatever the drag.
    cx.simulate_resize(size(px(900.), px(800.)));
    show_report(&store, cx);
    assert_eq!(panel_width(cx), Some(px(380.)));
    let from = edge(cx);
    drag(from, from - point(px(60.), px(0.)), cx);
    assert_eq!(panel_width(cx), Some(px(380.)));
    assert!(!files.layout().exists(), "nothing was written");

    // In a wider window it still opens at 46%.
    cx.simulate_resize(size(px(1500.), px(800.)));
    assert_eq!(panel_width(cx), Some(px(690.)));
}

#[gpui_kit::test]
fn the_report_expands_and_comes_back(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    show_report(&store, cx);
    assert_eq!(panel_width(cx), Some(px(580.)));

    // The shortcut: the whole content area, no page and no edge; again: back to its width.
    cx.simulate_keystrokes(TOGGLE);
    assert_eq!(panel_width(cx), Some(px(1000.)));
    assert_eq!(bounds(EDGE, cx), None, "nothing to drag");
    assert_eq!(bounds(PAGE, cx), None, "the report takes the page's place");
    cx.simulate_keystrokes(TOGGLE);
    assert_eq!(panel_width(cx), Some(px(580.)));
    assert_they_share_the_content_area(cx);

    // The button, the same.
    click(EXPAND, cx);
    assert_eq!(panel_width(cx), Some(px(1000.)));
    click(EXPAND, cx);
    assert_eq!(panel_width(cx), Some(px(580.)));

    // Esc brings an expanded report back, and only then closes it.
    cx.simulate_keystrokes(TOGGLE);
    assert_eq!(panel_width(cx), Some(px(1000.)));
    cx.simulate_keystrokes("escape");
    assert_eq!(panel_width(cx), Some(px(580.)));
    assert!(cx.read(|cx| store.read(cx).open_report().is_some()));
    cx.simulate_keystrokes("escape");
    assert_eq!(panel_width(cx), None);
    assert!(cx.read(|cx| store.read(cx).open_report().is_none()));

    // Closed while expanded: the expansion ends with it.
    show_report(&store, cx);
    cx.simulate_keystrokes(TOGGLE);
    assert!(is_expanded(&view, cx));
    store.update(cx, |store, cx| store.show_report(None, cx));
    assert!(!is_expanded(&view, cx), "ended as the report closed");
    assert_eq!(panel_width(cx), None);
    show_report(&store, cx);
    assert_eq!(panel_width(cx), Some(px(580.)));
}

#[gpui_kit::test]
fn with_no_report_the_shortcut_and_esc_do_nothing_new(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    cx.simulate_keystrokes(TOGGLE);
    assert!(!is_expanded(&view, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(panel_width(cx), None);
    show_report(&store, cx);
    assert_eq!(panel_width(cx), Some(px(580.)));
}

#[gpui_kit::test]
fn the_keys_work_from_the_composer_and_bring_its_focus_back(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    show_report(&store, cx);

    // Esc in the composer with the report at its width closes it, as it always did.
    focus_composer(&view, cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(panel_width(cx), None);
    assert!(composer_focused(&view, cx));

    // The shortcut from the composer expands it; the window takes the focus.
    show_report(&store, cx);
    cx.simulate_keystrokes(TOGGLE);
    assert_eq!(panel_width(cx), Some(px(1000.)));
    assert!(!composer_focused(&view, cx));

    // Each way back gives the composer its focus again: Esc, the shortcut, the button.
    cx.simulate_keystrokes("escape");
    assert!(!is_expanded(&view, cx));
    assert!(composer_focused(&view, cx), "after Esc");

    cx.simulate_keystrokes(TOGGLE);
    cx.simulate_keystrokes(TOGGLE);
    assert!(!is_expanded(&view, cx));
    assert!(composer_focused(&view, cx), "after the shortcut");

    cx.simulate_keystrokes(TOGGLE);
    click(EXPAND, cx);
    assert!(!is_expanded(&view, cx));
    assert!(composer_focused(&view, cx), "after the button");
}

#[gpui_kit::test]
fn a_page_change_brings_the_report_back_first(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    // Main among the rooms, for its shortcut.
    let main = serde_json::to_value(room(1, json!({}))).unwrap();
    store.update(cx, |store, cx| {
        store.take_state_for_tests(state(json!({ "rooms": [main] })), cx);
    });
    show_report(&store, cx);
    let expand = |cx: &mut VisualTestContext| {
        cx.simulate_keystrokes(TOGGLE);
        assert!(is_expanded(&view, cx));
    };

    // A click in the sidebar.
    expand(cx);
    click_id("needs", cx);
    assert_eq!(page(&view, cx), Page::Needs);
    assert!(!is_expanded(&view, cx), "Needs you");
    assert!(bounds(PAGE, cx).is_some());

    // A room's shortcut: with no room for the column beside the chat and the report, Needs you
    // is in the chat's place, and Main's chat comes back.
    expand(cx);
    cx.simulate_keystrokes(&primary("0"));
    assert!(matches!(page(&view, cx), Page::Room(_)));
    assert!(!is_expanded(&view, cx), "Main");
    assert!(composer_focused(&view, cx), "the chat's composer");

    // Settings' shortcut.
    expand(cx);
    cx.simulate_keystrokes(&primary(","));
    assert_eq!(page(&view, cx), Page::Settings);
    assert!(!is_expanded(&view, cx), "Settings");

    // The session ends: the sign-in screen shows, not under the report, and the report
    // waits beside the app.
    expand(cx);
    store.update(cx, Store::end_session);
    assert!(cx.read(|cx| store.read(cx).shows_sign_in()));
    assert!(!is_expanded(&view, cx), "sign-in");
    assert!(bounds(PAGE, cx).is_some());
    assert_eq!(panel_width(cx), None, "no report beside sign-in");
}

#[gpui_kit::test]
fn choosing_the_page_already_showing_brings_the_report_back(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    let main = serde_json::to_value(room(1, json!({}))).unwrap();
    store.update(cx, |store, cx| {
        store.take_state_for_tests(state(json!({ "rooms": [main] })), cx);
    });
    show_report(&store, cx);
    let main = page(&view, cx);
    assert!(matches!(main, Page::Room(_)));
    let expand = |cx: &mut VisualTestContext| {
        cx.simulate_keystrokes(TOGGLE);
        assert!(is_expanded(&view, cx));
    };

    // Main's row in the sidebar, on Main.
    expand(cx);
    click_id("ember", cx);
    assert_eq!(page(&view, cx), main);
    assert!(!is_expanded(&view, cx), "Main's row");
    assert!(composer_focused(&view, cx));

    // Main's shortcut, on Main.
    expand(cx);
    cx.simulate_keystrokes(&primary("0"));
    assert_eq!(page(&view, cx), main);
    assert!(!is_expanded(&view, cx), "Main's shortcut");

    // Needs you's row, on Needs you.
    click_id("needs", cx);
    expand(cx);
    click_id("needs", cx);
    assert_eq!(page(&view, cx), Page::Needs);
    assert!(!is_expanded(&view, cx), "Needs you's row");
    assert_eq!(panel_width(cx), Some(px(580.)));

    // A room shortcut with no room for it: nothing to open, but the report comes back.
    expand(cx);
    cx.simulate_keystrokes(&primary("9"));
    assert_eq!(page(&view, cx), Page::Needs);
    assert!(!is_expanded(&view, cx), "an unused room shortcut");
}

#[gpui_kit::test]
fn the_answer_toast_shows_while_the_report_is_expanded(cx: &mut TestAppContext) {
    let (store, _, cx) = open_window(ReportSize::default(), cx);
    show_report(&store, cx);
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(5), "Yes".into(), cx);
    });

    // At its width: over the page.
    let toast = bounds(TOAST, cx).expect("the toast shows");
    let page = bounds(PAGE, cx).expect("the page shows");
    assert!(page.contains(&toast.center()));

    // Expanded: under the report, outside the body (where a web view would cover it).
    cx.simulate_keystrokes(TOGGLE);
    let toast = bounds(TOAST, cx).expect("the toast still shows");
    let panel = bounds(PANEL, cx).expect("the panel shows");
    let body = bounds(BODY, cx).expect("the body shows");
    assert!(panel.contains(&toast.center()));
    assert!(body.bottom() <= toast.top(), "{body:?} above {toast:?}");

    // Undo works from there.
    click_id("undo", cx);
    assert!(cx.read(|cx| store.read(cx).answers().toast().is_none()));
    assert_eq!(bounds(TOAST, cx), None);
}

/// A ready report in the rich text fallback (there is no web view here) follows a drag and an
/// expansion like any other body.
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn the_rich_text_fallback_follows_the_panel(cx: &mut TestAppContext) {
    use super::RICH_TEXT;
    use super::edge::BAND;

    let backend = FakeBackend::new([]).with_file("/files/a.html", b"<h1>Sales</h1><p>a</p>");
    let (store, _, cx) = open_window_over(backend, ReportSize::default(), cx);
    let report = OpenReport {
        name: "a.html".to_owned(),
        path: "/files/a.html".to_owned(),
    };
    show(report, &store, cx);
    // The panel less its edge, at the default width, then 50 px wider.
    let space = Space::new(px(1260.), px(1000.));
    let at_first = PanelWidth::DEFAULT.pixels(space);
    assert_eq!(panel_width(cx), Some(at_first));
    assert_eq!(width_of(RICH_TEXT, cx), Some(at_first - BAND));

    let from = edge(cx);
    drag(from, from - point(px(50.), px(0.)), cx);
    let dragged = PanelWidth::from_pixels(at_first + px(50.), space)
        .unwrap()
        .pixels(space);
    assert_eq!(panel_width(cx), Some(dragged));
    assert_eq!(width_of(RICH_TEXT, cx), Some(dragged - BAND));

    cx.simulate_keystrokes(TOGGLE);
    assert_eq!(
        width_of(RICH_TEXT, cx),
        Some(space.content()),
        "the whole content area, with no edge"
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(width_of(RICH_TEXT, cx), Some(dragged - BAND));
}

#[gpui_kit::test]
fn the_pause_sheet_holds_the_keyboard(cx: &mut TestAppContext) {
    use crate::store::room_actions::Pausing;
    use crate::ui::pause::SHEET;

    let (store, view, cx) = open_window(ReportSize::default(), cx);
    show_report(&store, cx);
    let pause = |store: &Entity<Store>, cx: &mut VisualTestContext| {
        store.update(cx, |store, cx| {
            let pausing = Pausing {
                project: emberhut_api::ids::ProjectId::new(2),
                name: "Room 2".to_owned(),
            };
            store.set_pausing(Some(pausing), cx);
        });
        cx.run_until_parked();
        assert!(bounds(SHEET, cx).is_some(), "the sheet shows");
    };

    // Esc cancels the sheet, and leaves the report under it alone.
    pause(&store, cx);
    cx.simulate_keystrokes("escape");
    assert!(cx.read(|cx| store.read(cx).pausing().is_none()));
    assert_eq!(bounds(SHEET, cx), None);
    assert_eq!(panel_width(cx), Some(px(580.)), "the report stays open");

    // The window's shortcuts wait while it shows.
    pause(&store, cx);
    let before = page(&view, cx);
    cx.simulate_keystrokes(&primary(","));
    assert_eq!(page(&view, cx), before, "no page changes under the sheet");
    cx.simulate_keystrokes(TOGGLE);
    assert_eq!(panel_width(cx), Some(px(580.)), "nor the report");

    // Closed, the keyboard is the window's again.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(&primary(","));
    assert_eq!(page(&view, cx), Page::Settings);
}

#[gpui_kit::test]
fn a_refused_device_sees_its_screen_and_no_report(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    show_report(&store, cx);
    cx.simulate_keystrokes(TOGGLE);
    assert!(is_expanded(&view, cx));

    // The server stops letting this device in: its screen shows, with no report over or
    // beside it, and the keyboard isn't sent to a composer that isn't there.
    store.update(cx, Store::refuse_for_tests);
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| store.read(cx).gate()),
        crate::store::gate::Gate::NotYourDevice
    );
    assert!(!is_expanded(&view, cx));
    assert!(bounds(PAGE, cx).is_some());
    assert_eq!(panel_width(cx), None);
    assert!(!composer_focused(&view, cx));
    assert!(
        cx.read(|cx| store.read(cx).open_report().is_none()),
        "the report closed"
    );
}

/// The widths of the chat, the column beside it (or in its place) and the panel, those that
/// show (the page holds the chat and the column).
fn row_widths(cx: &mut VisualTestContext) -> (Option<Pixels>, Option<Pixels>, Option<Pixels>) {
    (width_of(CHAT, cx), width_of(COLUMN, cx), panel_width(cx))
}

/// Resizes the window to `width` and says whether the row fits in it beside the sidebar
/// (260 px): the page, the column and the panel end at the window's right edge.
fn fits_at(width: f32, cx: &mut VisualTestContext) -> bool {
    cx.simulate_resize(size(px(width), px(800.)));
    let (page, column, panel) = row_widths(cx);
    let row = [page, column, panel]
        .into_iter()
        .flatten()
        .fold(Pixels::ZERO, |row, width| row + width);
    row == px(width - 260.)
}

#[gpui_kit::test]
fn the_task_column_takes_what_the_page_and_the_report_leave(cx: &mut TestAppContext) {
    let (store, view, cx) = open_window(ReportSize::default(), cx);
    view.update_in(cx, |main, window, cx| {
        main.open_task(TaskId::new(5), window, cx);
    });
    cx.run_until_parked();

    // 1440 px: 1180 beside the sidebar. The column at 42%, the page the rest; with a report,
    // the page and the panel at their minimums first and the column the 440 px left.
    assert!(fits_at(1440., cx));
    assert_eq!(row_widths(cx), (Some(px(575.)), Some(px(605.)), None));
    show_report(&store, cx);
    assert!(fits_at(1440., cx));
    assert_eq!(
        row_widths(cx),
        (Some(px(360.)), Some(px(440.)), Some(px(380.)))
    );

    // 900 px: 640 beside the sidebar. No room for the column beside the chat and the report:
    // it takes the chat's place. Without the report, the chat keeps 360 and the column has 280.
    assert!(fits_at(900., cx));
    assert_eq!(row_widths(cx), (None, Some(px(260.)), Some(px(380.))));
    store.update(cx, |store, cx| store.show_report(None, cx));
    assert!(fits_at(900., cx));
    assert_eq!(row_widths(cx), (Some(px(360.)), Some(px(280.)), None));

    // 600 px: the column in the chat's place, and nothing runs past the window, report or not.
    assert!(fits_at(600., cx));
    assert_eq!(row_widths(cx), (None, Some(px(340.)), None));
    show_report(&store, cx);
    assert!(fits_at(600., cx));
    assert_eq!(width_of(CHAT, cx), None);

    // The task stayed open: with room again, its page is beside the chat.
    store.update(cx, |store, cx| store.show_report(None, cx));
    assert!(fits_at(1440., cx));
    assert_eq!(width_of(COLUMN, cx), Some(px(605.)));
    assert_eq!(page(&view, cx), Page::Task(TaskId::new(5)));
}

#[gpui_kit::test]
fn in_a_narrow_window_the_column_takes_the_chats_place_until_back(cx: &mut TestAppContext) {
    let (_, view, cx) = open_window(ReportSize::default(), cx);
    // 700 px: 440 beside the sidebar, less than the chat's 360 and the column's 280.
    cx.simulate_resize(size(px(700.), px(800.)));
    // More starts folded: unfolded, its Settings row shows.
    click_id("more", cx);
    click_id("settings", cx);
    assert_eq!(page(&view, cx), Page::Settings);
    assert_eq!(
        row_widths(cx),
        (None, Some(px(440.)), None),
        "Settings shows, in the chat's place"
    );

    cx.simulate_keystrokes(&primary("["));
    assert!(matches!(page(&view, cx), Page::Room(_)), "the chat alone");
    assert_eq!(
        row_widths(cx),
        (Some(px(440.)), None, None),
        "Back brings the chat back"
    );
}
