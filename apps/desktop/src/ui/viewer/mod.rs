//! The file viewer: one file (a chat attachment, a task's result) large, fetched whole first. A
//! picture zooms (a pinch, a double-click, ⌘+ ⌘− ⌘0) and scrolls, a PDF shows its pages one
//! after another fitted to the width with the page the reader is on (`3 / 12`), a note or a
//! text file reads as selectable text that zooms, a video plays and a deck, document or sheet
//! previews (macOS: `AVKit` and Quick Look; elsewhere they open in their default app).
//!
//! In a main window it is a sheet over the window at 85% of its size: its name and Open in a
//! window on top, its uses below (Share…, Save to Downloads ⌘S, Save As…, Open in its app,
//! Copy), the zoom, and Done (Return; Esc too). ⌃⌘F (F11 off the Mac) moves it to a window of
//! its own and makes that full screen; in its own window ⌃⌘F goes in and out of full screen,
//! and Esc leaves it. A report doesn't open here: it opens in the panel beside the page.

mod bars;
mod body;
mod native;
mod note;
pub(crate) mod own_window;
pub(crate) mod pages;
mod pdf_view;
mod picture;
mod zoom;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, Bounds, Entity, EventEmitter, FocusHandle, KeyBinding, Pixels, RenderImage,
    ScrollHandle, Window, actions, canvas, div, point, px, relative,
};

use self::body::Body;
use self::zoom::Zoom;
use super::a11y::Accessible as _;
use super::files::status_line;
use super::palette::pal;
use crate::store::files::{Kind, RemoteFile};
use crate::store::{OpenReport, Store};

actions!(
    viewer,
    [
        /// Done: closes the viewer sheet.
        CloseViewer,
        /// Saves the file shown into Downloads.
        SaveToDownloads,
        /// Zooms in.
        ZoomIn,
        /// Zooms out.
        ZoomOut,
        /// Back to the size that fits.
        ZoomToFit,
        /// The viewer full screen: in a window of its own first.
        ViewerFullScreen,
        /// Esc in the viewer: closes the sheet, or leaves full screen in its own window.
        ViewerEscape,
    ]
);

/// Opens a file in the viewer of the window it is dispatched in (a report in the panel beside
/// the page instead).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = viewer, no_json)]
pub(crate) struct OpenInViewer {
    /// The file.
    pub(crate) file: RemoteFile,
}

/// The key context the viewer's shortcuts work in: they win over the window's own (⌘0 is Main
/// elsewhere) while the viewer has the keyboard.
const CONTEXT: &str = "FileViewer";

/// The viewer's shortcuts: ⌘ on the Mac, Ctrl elsewhere.
pub(crate) fn bind_keys(cx: &mut App) {
    #[cfg(target_os = "macos")]
    let keys = [
        KeyBinding::new("cmd-s", SaveToDownloads, Some(CONTEXT)),
        KeyBinding::new("cmd-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd--", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("cmd-0", ZoomToFit, Some(CONTEXT)),
        KeyBinding::new("ctrl-cmd-f", ViewerFullScreen, Some(CONTEXT)),
    ];
    #[cfg(not(target_os = "macos"))]
    let keys = [
        KeyBinding::new("ctrl-s", SaveToDownloads, Some(CONTEXT)),
        KeyBinding::new("ctrl-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("ctrl-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("ctrl--", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("ctrl-0", ZoomToFit, Some(CONTEXT)),
        KeyBinding::new("f11", ViewerFullScreen, Some(CONTEXT)),
    ];
    cx.bind_keys(keys);
    cx.bind_keys([
        KeyBinding::new("enter", CloseViewer, Some(CONTEXT)),
        KeyBinding::new("escape", ViewerEscape, Some(CONTEXT)),
    ]);
}

/// The viewer closed (its sheet), so the window takes the keyboard back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Closed;

/// The viewer: a sheet in a main window (empty while nothing is viewed), or a window's whole.
pub(crate) struct FileViewer {
    store: Entity<Store>,
    focus: FocusHandle,
    /// In a window of its own: no Done, no Open in a window.
    windowed: bool,
    /// The file shown, if any.
    open: Option<Open>,
    /// How many files it opened, so a load that finishes after another file opened is dropped.
    opened: u64,
}

/// A file being viewed.
struct Open {
    file: RemoteFile,
    kind: Kind,
    zoom: Zoom,
    body: Body,
    /// The body's bounds as last drawn, to fit the file in.
    area: Bounds<Pixels>,
    scroll: ScrollHandle,
    /// Which open it is (see [`FileViewer::opened`]).
    number: u64,
}

impl EventEmitter<Closed> for FileViewer {}

impl FileViewer {
    /// Lets go of the platform's view of a video or document, if one is up. It is native,
    /// drawn above whatever the window draws, so the app lock's cover drops it (a video stops)
    /// rather than only leaving it undrawn; the next frame that shows the file makes it again.
    pub(crate) fn let_go_of_native(&mut self) {
        if let Some(Open {
            body: Body::Media { native, .. },
            ..
        }) = &mut self.open
        {
            *native = None;
        }
    }

    /// A viewer over `store` with nothing open, for a main window.
    pub(crate) fn new(store: Entity<Store>, cx: &mut Context<'_, Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            focus: cx.focus_handle(),
            windowed: false,
            open: None,
            opened: 0,
        }
    }

    /// A viewer showing `file`, as the whole of a window of its own.
    pub(crate) fn in_window(
        store: Entity<Store>,
        file: RemoteFile,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // It counts for where the app is, as the other windows do (polling, the lock).
        crate::shell::presence::follow(window, &store, cx);
        let mut viewer = Self::new(store, cx);
        viewer.windowed = true;
        // The size the user gives it is the next one's.
        cx.observe_window_bounds(window, |_, window, cx| {
            own_window::note_size(window.window_bounds(), cx);
        })
        .detach();
        viewer.show(file, window, cx);
        viewer
    }

    /// Whether a file is shown.
    #[must_use]
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Shows `file`, fetched first, and takes the keyboard.
    pub(crate) fn show(
        &mut self,
        file: RemoteFile,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_quietly(cx);
        self.opened += 1;
        let kind = file.kind();
        self.open = Some(Open {
            zoom: Zoom::fitted(kind),
            kind,
            body: Body::Loading,
            area: Bounds::default(),
            scroll: ScrollHandle::new(),
            number: self.opened,
            file: file.clone(),
        });
        body::load(&self.store, file, kind, self.opened, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Shows `file`, which a note shown here links to: a report goes to the report panel
    /// beside the page, anything else shows here in its place.
    fn follow(&mut self, file: RemoteFile, window: &mut Window, cx: &mut Context<'_, Self>) {
        if file.kind() == Kind::Report {
            let report = OpenReport {
                name: file.name,
                path: file.path,
            };
            self.store
                .update(cx, |store, cx| store.show_report(Some(report), cx));
            return;
        }
        self.show(file, window, cx);
    }

    /// A load for open `number` finished with `body`: shown, unless another file opened since.
    fn loaded(&mut self, number: u64, body: Body, cx: &mut Context<'_, Self>) {
        match &mut self.open {
            Some(open) if open.number == number => {
                open.body = body;
                cx.notify();
            }
            _ => body.free(cx),
        }
    }

    /// Done: the sheet closes (a window of its own closes itself).
    fn done(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.windowed {
            window.remove_window();
            return;
        }
        self.close_quietly(cx);
        cx.emit(Closed);
        cx.notify();
    }

    /// Forgets the file shown, freeing what it held.
    fn close_quietly(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(open) = self.open.take() {
            open.body.free(cx);
        }
    }

    /// Esc: closes the sheet; in a window of its own, leaves full screen.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.windowed {
            self.done(window, cx);
        } else if window.is_fullscreen() {
            window.toggle_fullscreen();
        }
    }

    /// ⌃⌘F: full screen, in a window of its own (the sheet moves to one first).
    fn full_screen(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.windowed {
            window.toggle_fullscreen();
            return;
        }
        self.move_to_own_window(true, window, cx);
    }

    /// Open in a window: the file moves from the sheet to a window of its own.
    fn move_to_own_window(
        &mut self,
        full_screen: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(file) = self.open.as_ref().map(|open| open.file.clone()) else {
            return;
        };
        self.done(window, cx);
        own_window::open(&self.store, file, full_screen, cx);
    }

    /// Zooms by `by` (`None`: back to fit).
    fn zoom(&mut self, by: Option<f32>, cx: &mut Context<'_, Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        if !open.kind.zooms() {
            return;
        }
        match by {
            Some(by) => open.zoom.step(by),
            None => open.zoom.fit(),
        }
        open.centre();
        cx.notify();
    }

    /// The file's uses, for the bars.
    fn file(&self) -> Option<&RemoteFile> {
        self.open.as_ref().map(|open| &open.file)
    }
}

impl Open {
    /// Scrolls a zoomed picture so its middle shows (as a zoom from the middle does).
    fn centre(&self) {
        if let Body::Picture(image) = &self.body {
            let shown = picture::shown_size(image, self.area.size, self.zoom);
            let x = (shown.width - self.area.size.width).max(px(0.)) / 2.;
            let y = (shown.height - self.area.size.height).max(px(0.)) / 2.;
            self.scroll.set_offset(point(-x, -y));
        }
    }
}

/// The sheet's size in a window `window` big: 85% of it, at least 760 by 560 where it fits.
fn sheet_size(window: gpui_kit::Size<Pixels>) -> (Pixels, Pixels) {
    let fit = |share: Pixels, least: f32, room: Pixels| {
        share.max(px(least)).min((room - px(40.)).max(px(320.)))
    };
    (
        fit(window.width * 0.85, 760., window.width),
        fit(window.height * 0.85, 560., window.height),
    )
}

/// Frees `image`'s tiles in every window once no window is being drawn.
fn free_image(image: Arc<RenderImage>, cx: &mut App) {
    cx.defer(move |cx| cx.drop_image(image, None));
}

impl Render for FileViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        if self.open.is_none() {
            return div().into_any_element();
        }
        // A file's own window is covered while the app is locked, as every window is; a
        // video's or document's native view goes with it (the sheet's goes with its window's
        // cover, `MainWindow`'s render).
        if self.windowed
            && let Some(cover) = crate::ui::lock::cover(&p, cx)
        {
            self.let_go_of_native();
            return cover.into_any_element();
        }
        let entity = cx.entity();
        let body = body::render(self, window, cx);
        let measure = canvas(
            move |bounds, _, cx| {
                entity.update(cx, |this, cx| {
                    if let Some(open) = &mut this.open
                        && open.area != bounds
                    {
                        open.area = bounds;
                        cx.notify();
                    }
                });
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();
        let content = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseViewer, window, cx| this.done(window, cx)))
            // A file a note links to opens here (a report in the main window's panel), in a
            // window of its own as in the sheet.
            .on_action(cx.listener(|this, open: &OpenInViewer, window, cx| {
                this.follow(open.file.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ViewerEscape, window, cx| this.escape(window, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom(Some(zoom::IN), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom(Some(zoom::OUT), cx)))
            .on_action(cx.listener(|this, _: &ZoomToFit, _, cx| this.zoom(None, cx)))
            .on_action(
                cx.listener(|this, _: &ViewerFullScreen, window, cx| this.full_screen(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SaveToDownloads, _, cx| {
                if let Some(file) = this.file().cloned() {
                    bars::save_to_downloads(&this.store, file, cx);
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(p.surface)
            .text_color(p.text)
            .when(!self.windowed, |d| d.child(bars::top(self, &p, cx)))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(measure)
                    .child(body),
            )
            .child(bars::bottom(self, &p, cx));
        if self.windowed {
            return div()
                .relative()
                .size_full()
                .child(content)
                .children(status_line::view(&self.store, &p, cx))
                .into_any_element();
        }
        let (width, height) = sheet_size(window.viewport_size());
        let name = self.file().map_or_default(|file| file.name.clone());
        div()
            .id("viewer-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui_kit::black().opacity(0.35))
            // Nothing under the sheet answers the pointer: no hover, no scrolling, and a click
            // beside the sheet does nothing, as beside a Mac sheet.
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("viewer-sheet")
                    .read_as(gpui_kit::Role::Dialog, name)
                    .w(width)
                    .h(height)
                    .max_w(relative(1.))
                    .rounded(px(12.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(p.border)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(content),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::ui::palette::Pal;

    #[gpui_kit::test]
    fn a_file_that_could_run_is_only_offered_in_its_folder(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_download(Ok(b"MZ".to_vec()));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let (root, opened) = (
            tempfile::tempdir().unwrap(),
            Rc::new(RefCell::new(Vec::<PathBuf>::new())),
        );
        let store = cx.new(|_| {
            let mut store = Store::for_tests(shared);
            store.keep_files_in(root.path(), &opened);
            store
        });
        let file = RemoteFile::new("/uploads/setup.exe", "setup.exe");
        let window = cx.add_window(|window, cx| FileViewer::in_window(store, file, window, cx));
        cx.run_until_parked();
        let program = window
            .update(cx, |viewer, _, _| {
                matches!(
                    viewer.open.as_ref().map(|open| &open.body),
                    Some(Body::Program)
                )
            })
            .unwrap();
        assert!(program, "neither previewed nor opened");
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui_kit::size(px(900.), px(700.)));
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("viewer-show-in-folder").is_some());
            assert!(window.try_find(body::PROGRAM_NOTE).is_some(), "it says why");
            assert!(
                window.try_find("viewer-ShowInFolder").is_some(),
                "in the bar"
            );
            assert!(window.try_find("viewer-OpenInApp").is_none(), "never Open");
        });
        assert!(opened.borrow().is_empty(), "nothing opened or previewed");
    }

    /// A viewer in a window of its own showing the note `text` (`notes.md`), 900 × 700.
    fn note_window(text: &str, cx: &mut TestAppContext) -> VisualTestContext {
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_download(Ok(text.as_bytes().to_vec()));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let root = tempfile::tempdir().unwrap();
        let opened = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
        let store = cx.new(|_| {
            let mut store = Store::for_tests(shared);
            store.keep_files_in(root.path(), &opened);
            store
        });
        let file = RemoteFile::new("/uploads/notes.md", "notes.md");
        let window = cx.add_window(|window, cx| FileViewer::in_window(store, file, window, cx));
        cx.run_until_parked();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui_kit::size(px(900.), px(700.)));
        cx.run_until_parked();
        cx.update(Window::render_frame);
        cx
    }

    /// Clicks the start of the note's first line, where its link is.
    fn click_the_link(cx: &mut VisualTestContext) {
        // The note's padding is 24 by 18; the link's first letters come right after.
        cx.simulate_click(point(px(30.), px(28.)), gpui_kit::Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_link_in_a_note_opens_only_as_the_apps_links_do(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let mut web = note_window("[Example page here](https://example.com/plan)", cx);
        click_the_link(&mut web);
        assert_eq!(
            web.opened_url().as_deref(),
            Some("https://example.com/plan"),
            "the click lands on the link, and a web address opens in the browser"
        );
        for shut in [
            "[Example file here](file:///Applications/Calculator.app)",
            "[Example share here](smb://example.com/share)",
            "[Example app here](x-example-app://open)",
        ] {
            let mut shown = note_window(shut, cx);
            click_the_link(&mut shown);
            assert_eq!(
                shown.opened_url().as_deref(),
                Some("https://example.com/plan"),
                "nothing else opened: {shut}"
            );
        }
    }

    #[gpui_kit::test]
    fn a_file_a_note_links_to_opens_in_the_viewers_own_window(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let mut shown = note_window("[The plan](/uploads/plan.pdf)", cx);
        let linked = RemoteFile::new("/uploads/plan.pdf", "plan.pdf");
        shown.dispatch_action(OpenInViewer {
            file: linked.clone(),
        });
        shown.run_until_parked();
        let open = shown.update(|window, cx| {
            window
                .root::<FileViewer>()
                .flatten()
                .and_then(|viewer| viewer.read(cx).file().cloned())
        });
        assert_eq!(open, Some(linked));
    }

    /// A store whose downloads come from `backend`, its files kept in `root`.
    #[cfg(not(target_os = "macos"))]
    fn store_over(
        backend: FakeBackend,
        root: &std::path::Path,
        cx: &mut TestAppContext,
    ) -> Entity<Store> {
        let shared = Arc::new(backend) as Arc<dyn crate::backend::Backend>;
        let opened = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
        cx.new(|cx| {
            let mut store = Store::for_tests(shared);
            store.keep_files_in(root, &opened);
            store.take_state_for_tests(crate::fixtures::state(serde_json::json!({})), cx);
            store
        })
    }

    /// The video `clip.mov`, its bytes made up.
    #[cfg(not(target_os = "macos"))]
    fn video(backend: &FakeBackend) -> RemoteFile {
        backend.push_download(Ok(b"not really a video".to_vec()));
        RemoteFile::new("/uploads/clip.mov", "clip.mov")
    }

    /// Locks the app (or opens it) and draws a frame.
    #[cfg(not(target_os = "macos"))]
    fn lock(locked: bool, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            cx.set_global(crate::lock::Lock::at_launch(true, !locked));
            window.render_frame(cx);
        });
    }

    /// A video playing in a file's own window stops and goes while the app is locked (its view
    /// is native, drawn over the lock's cover), and comes back once it is unlocked.
    #[cfg(not(target_os = "macos"))]
    #[gpui_kit::test]
    fn a_locked_files_window_holds_no_native_view(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        native::stand_in::turn_on();
        let (backend, root) = (FakeBackend::new([]), tempfile::tempdir().unwrap());
        let file = video(&backend);
        let store = store_over(backend, root.path(), cx);
        let window = cx.add_window(|window, cx| FileViewer::in_window(store, file, window, cx));
        cx.run_until_parked();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui_kit::size(px(900.), px(700.)));
        cx.update(Window::render_frame);
        assert_eq!(native::stand_in::up(), 1, "the video plays");

        lock(true, &mut cx);
        assert_eq!(native::stand_in::up(), 0, "nothing native under the cover");
        cx.update(|window, _| assert!(window.try_find("unlock").is_some(), "the cover"));

        lock(false, &mut cx);
        assert_eq!(native::stand_in::up(), 1, "back once unlocked");
    }

    /// The same for the viewer's sheet over a main window, whose cover takes the sheet's place.
    #[cfg(not(target_os = "macos"))]
    #[gpui_kit::test]
    fn a_locked_main_windows_sheet_holds_no_native_view(cx: &mut TestAppContext) {
        use crate::ui::report::ReportSize;
        use crate::ui::window::MainWindow;

        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        native::stand_in::turn_on();
        let (backend, root) = (FakeBackend::new([]), tempfile::tempdir().unwrap());
        let file = video(&backend);
        let store = store_over(backend, root.path(), cx);
        let (_view, cx) = cx.add_window_view(|window, cx| {
            MainWindow::new(store.clone(), ReportSize::default(), window, cx)
        });
        cx.simulate_resize(gpui_kit::size(px(1200.), px(800.)));
        cx.dispatch_action(OpenInViewer { file });
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert_eq!(native::stand_in::up(), 1, "the video plays in the sheet");

        lock(true, cx);
        assert_eq!(native::stand_in::up(), 0, "nothing native under the cover");

        lock(false, cx);
        assert_eq!(native::stand_in::up(), 1, "back once unlocked");
    }

    #[test]
    fn the_sheet_is_most_of_the_window_but_fits_in_it() {
        let (width, height) = sheet_size(gpui_kit::size(px(1400.), px(1000.)));
        assert_eq!((width, height), (px(1190.), px(850.)));
        let (width, height) = sheet_size(gpui_kit::size(px(900.), px(600.)));
        assert_eq!((width, height), (px(765.), px(560.)), "at least 560 tall");
        let (width, _) = sheet_size(gpui_kit::size(px(700.), px(600.)));
        assert_eq!(width, px(660.), "never wider than the window leaves");
    }
}
