//! The shell over real windows on GPUI's test platform.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::ids::{RoomId, TaskId};
use gpui_kit::prelude::*;
use gpui_kit::{Bounds, Entity, Pixels, TestAppContext, Window, WindowBounds, div};

use super::unseen::{self, Unseen};
use super::windows::{self, Opening};
use super::{WindowKind, go};
use crate::backend::Backend;
use crate::backend::fake::FakeBackend;
use crate::links::Target;
use crate::store::files::RemoteFile;
use crate::store::presence::Presence;
use crate::store::{Page, Store};
use crate::ui::palette::Pal;
use crate::ui::report::ReportSize;

/// A window of its own's stand-in view.
struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
    }
}

fn setup(cx: &mut TestAppContext) -> Entity<Store> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = Arc::new(FakeBackend::new([])) as Arc<dyn Backend>;
    let store = cx.new(|_| Store::for_tests(backend));
    cx.update(|cx| windows::install(store.clone(), ReportSize::default(), cx));
    store
}

fn open_main(cx: &mut TestAppContext) {
    cx.update(|cx| windows::open_main(Opening::default(), cx).map(drop))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn links_go_to_the_main_window_and_a_room_keeps_the_column(cx: &mut TestAppContext) {
    let store = setup(cx);
    open_main(cx);
    open_main(cx);
    assert_eq!(cx.update(|cx| windows::main_count(cx)), 2, "New Window");
    cx.update(|cx| go(&Target::Page(Page::Tasks), cx));
    cx.update(|cx| go(&Target::Chat(Some(RoomId::new(2))), cx));
    cx.run_until_parked();
    let room = store.read_with(cx, |store, _| store.room());
    assert_eq!(room, RoomId::new(2));
    assert_eq!(
        front_column(cx),
        Some(Page::Tasks),
        "the page stays beside the chat"
    );
    cx.update(|cx| go(&Target::Task(TaskId::new(7)), cx));
    cx.run_until_parked();
    assert_eq!(
        front_column(cx),
        Some(Page::Task(TaskId::new(7))),
        "a task's link opens its page, over the one shown"
    );
    assert!(store.read_with(cx, |store, _| store.shows_page(Page::Task(TaskId::new(7)))));
}

/// What the front main window's column shows.
fn front_column(cx: &mut TestAppContext) -> Option<Page> {
    cx.update(|cx| {
        let handle = windows::front_main(cx)?;
        handle
            .update(cx, |_, window, cx| {
                crate::ui::column::of(window, cx).and_then(|column| column.read(cx).current())
            })
            .ok()
            .flatten()
    })
}

#[gpui_kit::test]
fn a_link_with_no_main_window_open_opens_one(cx: &mut TestAppContext) {
    let store = setup(cx);
    cx.update(|cx| go(&Target::Chat(Some(RoomId::new(3))), cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| windows::main_count(cx)), 1);
    assert_eq!(store.read_with(cx, |store, _| store.room()), RoomId::new(3));
}

#[gpui_kit::test]
fn a_window_of_its_own_opens_once_and_only_with_a_view(cx: &mut TestAppContext) {
    setup(cx);
    open_main(cx);
    let count = |cx: &mut TestAppContext| cx.update(|cx| cx.windows().len());
    assert!(!cx.update(|cx| windows::open_single(WindowKind::Diary, cx)));
    assert_eq!(cx.update(|cx| windows::available(cx)), []);
    cx.update(|cx| {
        windows::register_view(
            WindowKind::Diary,
            Rc::new(|_, _, cx| cx.new(|_| Blank).into()),
            cx,
        );
    });
    assert_eq!(cx.update(|cx| windows::available(cx)), [WindowKind::Diary]);
    assert!(cx.update(|cx| windows::open_single(WindowKind::Diary, cx)));
    assert_eq!(count(cx), 2);
    assert!(cx.update(|cx| windows::open_single(WindowKind::Diary, cx)));
    assert_eq!(count(cx), 2, "brought forward, not opened again");
    // A link to a window with no view brings the main window forward instead.
    cx.update(|cx| go(&Target::Window(WindowKind::Browser), cx));
    assert_eq!(count(cx), 2);
}

#[gpui_kit::test]
fn with_every_main_window_closed_nothing_is_on_screen(cx: &mut TestAppContext) {
    let store = setup(cx);
    open_main(cx);
    let presence = |cx: &mut TestAppContext| store.read_with(cx, |store, _| store.presence());
    assert_eq!(presence(cx), Presence::Behind);
    let open = cx.update(|cx| cx.windows());
    for handle in open {
        cx.update(|cx| handle.update(cx, |_, window, _| window.remove_window()))
            .unwrap();
    }
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| windows::main_count(cx)), 0);
    assert_eq!(presence(cx), Presence::Hidden, "nothing polls");
    cx.update(windows::reopen);
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| windows::main_count(cx)),
        1,
        "the Dock reopens one"
    );
}

#[gpui_kit::test]
fn talk_in_a_window_of_its_own_brings_a_main_window_forward(cx: &mut TestAppContext) {
    let store = setup(cx);
    cx.update(|cx| {
        windows::register_view(
            WindowKind::Sessions,
            Rc::new(|_, _, cx| cx.new(|_| Blank).into()),
            cx,
        );
    });
    assert!(cx.update(|cx| windows::open_single(WindowKind::Sessions, cx)));
    let own = cx.update(|cx| cx.windows()[0]);
    assert!(!cx.update(|cx| windows::is_main(own, cx)));
    store.update(cx, |store, cx| store.start_message("About the plan: ", cx));
    cx.update(|cx| own.update(cx, |_, window, cx| super::talk_shown(window, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| windows::main_count(cx)),
        1,
        "a main window opens to show the words"
    );
    let main = cx.update(|cx| windows::front_main(cx)).unwrap();
    assert!(cx.update(|cx| windows::is_main(main, cx)));
    cx.update(|cx| main.update(cx, |_, window, cx| super::talk_shown(window, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| windows::main_count(cx)),
        1,
        "from a main window, its own chat shows them"
    );
}

#[gpui_kit::test]
fn a_window_of_its_own_reopens_where_it_was_left_and_open_ones_reopen(cx: &mut TestAppContext) {
    use gpui_kit::{Bounds, WindowBounds, point, px, size};

    let store = setup(cx);
    cx.update(|cx| {
        cx.set_global(crate::prefs::Kept::load(None));
        for kind in [WindowKind::Diary, WindowKind::QuickNote] {
            windows::register_view(kind, Rc::new(|_, _, cx| cx.new(|_| Blank).into()), cx);
        }
    });
    let left = Bounds::new(point(px(120.), px(90.)), size(px(600.), px(700.)));
    cx.update(|cx| {
        crate::prefs::Kept::note_own_window(cx, "diary", WindowBounds::Windowed(left));
    });
    assert!(cx.update(|cx| windows::open_single(WindowKind::Diary, cx)));
    assert!(cx.update(|cx| windows::open_single(WindowKind::QuickNote, cx)));
    cx.run_until_parked();
    let diary = cx.update(|cx| cx.windows()[0]);
    let opened = cx
        .update(|cx| diary.update(cx, |_, window, _| window.bounds()))
        .unwrap();
    assert_eq!(opened.size, left.size, "at the size it was left");
    cx.update(super::restore::note_open);
    let open = cx.update(|cx| crate::prefs::Kept::get(cx).own_windows.open);
    assert_eq!(open, ["diary"], "a quick note starts afresh");
    for handle in cx.update(|cx| cx.windows()) {
        cx.update(|cx| handle.update(cx, |_, window, _| window.remove_window()))
            .unwrap();
    }
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| windows::open_singles(cx)), []);
    // At launch the app connects, then asks this device to sign in: nothing reopens over that.
    cx.update(super::restore::reopen);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| windows::open_singles(cx)), [], "connecting");
    store.update(cx, |store, cx| {
        store.take_state_for_tests(
            crate::fixtures::state(
                serde_json::json!({"auth": {"mode": "report", "bound": true, "devices": []}}),
            ),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| windows::open_singles(cx)), [], "signed out");
    store.update(cx, Store::sign_in_later);
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| windows::open_singles(cx)),
        [WindowKind::Diary],
        "once the app is open"
    );
    store.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows()).len(), 1, "and only once");
}

/// A window the app has open, as a test sees it.
struct Opened {
    /// Where it is.
    bounds: Bounds<Pixels>,
    /// Whether it is full screen.
    full_screen: bool,
}

/// Opens every kind of window there is, as a run of the app may: the main window on the first
/// display (as `--display 0` asks), each window of its own, each asked for again and the main
/// window brought forward (as a link does), and a file in its own window, full screen once it
/// has settled. What became of each.
fn open_every_window(store: &Entity<Store>, cx: &mut TestAppContext) -> Vec<Opened> {
    cx.update(|cx| {
        for kind in WindowKind::ALL {
            windows::register_view(kind, Rc::new(|_, _, cx| cx.new(|_| Blank).into()), cx);
        }
        let display = cx.displays().first().map(|display| display.id());
        let size = gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(760.));
        let opening = Opening {
            bounds: Some(WindowBounds::Windowed(Bounds::centered(display, size, cx))),
            display,
            background: false,
        };
        windows::open_main(opening, cx).unwrap();
        for kind in WindowKind::ALL {
            assert!(windows::open_single(kind, cx), "{kind:?}");
        }
        for kind in WindowKind::ALL {
            go(&Target::Window(kind), cx);
        }
        go(&Target::Chat(None), cx);
        let file = RemoteFile::new("/uploads/plan.txt", "plan.txt");
        crate::ui::viewer::own_window::open(store, file, true, cx);
    });
    // Past the file's window settling, when it goes full screen.
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let opened = cx.update(|cx| cx.windows());
    assert_eq!(
        opened.len(),
        1 + WindowKind::ALL.len() + 1,
        "the main window, each of its own, the file's"
    );
    opened
        .into_iter()
        .map(|handle| {
            cx.update(|cx| {
                handle.update(cx, |_, window, _| Opened {
                    bounds: window.bounds(),
                    full_screen: window.is_fullscreen(),
                })
            })
            .unwrap()
        })
        .collect()
}

/// The displays' bounds.
fn displays(cx: &mut TestAppContext) -> Vec<Bounds<Pixels>> {
    cx.update(|cx| {
        cx.displays()
            .iter()
            .map(|display| display.bounds())
            .collect()
    })
}

/// Whether each of `opened` is on one of `displays`.
fn all_on_a_display(opened: &[Opened], displays: &[Bounds<Pixels>]) -> bool {
    opened.iter().all(|window| {
        displays
            .iter()
            .any(|display| window.bounds.intersects(display))
    })
}

/// In an unseen run no window comes forward or goes full screen, each opens without the focus,
/// and (off the Mac, where a window out of sight is one beside the displays) none is on a
/// display. On a Mac a window out of sight stays on a display, hidden until it is below the
/// desktop: its options say hidden here, and `unseen`'s Mac test checks how it is kept.
#[gpui_kit::test]
fn in_an_unseen_run_no_window_shows_nor_comes_forward(cx: &mut TestAppContext) {
    let store = setup(cx);
    cx.update(|cx| unseen::mark(Unseen::OutOfSight, cx));
    let displays = displays(cx);
    assert!(!displays.is_empty(), "the test platform has a display");
    let opened = open_every_window(&store, cx);
    #[cfg(not(target_os = "macos"))]
    for window in &opened {
        for display in &displays {
            assert!(
                !window.bounds.intersects(display),
                "{:?} is on {display:?}",
                window.bounds
            );
        }
    }
    assert!(opened.iter().all(|window| !window.full_screen));
    assert_eq!(
        cx.update(|cx| cx.active_window()),
        None,
        "no window was brought forward"
    );
    let options = cx.update(|cx| unseen::options(gpui_kit::WindowOptions::default(), cx));
    assert!(!options.focus, "every window opens without the focus");
    #[cfg(target_os = "macos")]
    assert!(!options.show, "hidden until it is below the desktop");
    assert_eq!(
        store.read_with(cx, |store, _| store.presence()),
        Presence::Front,
        "the run counts as in front, so rooms are read"
    );
}

#[gpui_kit::test]
fn in_a_normal_run_the_windows_are_on_a_display_and_come_forward(cx: &mut TestAppContext) {
    let store = setup(cx);
    let displays = displays(cx);
    let opened = open_every_window(&store, cx);
    assert!(all_on_a_display(&opened, &displays));
    assert!(
        opened.iter().any(|window| window.full_screen),
        "the file's window went full screen"
    );
    assert!(
        cx.update(|cx| cx.active_window()).is_some(),
        "the main window came forward"
    );
    let options = cx.update(|cx| unseen::options(gpui_kit::WindowOptions::default(), cx));
    assert!(options.focus && options.show);
}

#[gpui_kit::test]
fn an_unseen_run_on_screen_stays_on_a_display_without_the_focus(cx: &mut TestAppContext) {
    let store = setup(cx);
    cx.update(|cx| unseen::mark(Unseen::OnScreen, cx));
    let displays = displays(cx);
    let opened = open_every_window(&store, cx);
    assert!(all_on_a_display(&opened, &displays));
    assert_eq!(cx.update(|cx| cx.active_window()), None);
    let options = cx.update(|cx| unseen::options(gpui_kit::WindowOptions::default(), cx));
    assert!(!options.focus && options.show);
}
