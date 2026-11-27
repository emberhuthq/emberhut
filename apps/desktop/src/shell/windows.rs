//! The app's windows over its one store: any number of main windows (File › New Window), each
//! with its own sidebar, all showing the store's one column (its page, the pages under it, the
//! task pages over it), and the windows of their own ([`WindowKind`]), one each.
//!
//! A window of its own opens through the view its feature registers ([`register_view`]); a
//! kind with no view in this build has no window, and a link to it brings the main window
//! forward instead.

use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Context as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, Bounds, DisplayId, Entity, Global, Pixels, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowId, WindowOptions, point, px, size,
};

use super::WindowKind;
use crate::brand::{APP_ID, APP_NAME};
use crate::prefs::{self, Kept};
use crate::store::Store;
use crate::ui::look;
use crate::ui::report::ReportSize;
use crate::ui::window::MainWindow;

/// Builds a window of its own's view over the store.
pub(crate) type ViewBuilder = Rc<dyn Fn(Entity<Store>, &mut Window, &mut App) -> AnyView>;

/// How far a new main window sits from the one it opened from.
const CASCADE: f32 = 24.;

/// The windows the app has open.
pub(crate) struct Windows {
    store: Entity<Store>,
    /// The main windows, the one used last first.
    mains: Vec<(AnyWindowHandle, WeakEntity<MainWindow>)>,
    /// The windows of their own that are open.
    singles: HashMap<WindowKind, AnyWindowHandle>,
    /// The views the windows of their own are made of, by kind.
    views: HashMap<WindowKind, ViewBuilder>,
    /// The report panel's size read at launch, for the first main window; the next ones start
    /// at the panel's default width.
    first_report_size: Option<ReportSize>,
}

impl Global for Windows {}

/// How a main window opens.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Opening {
    /// Its bounds; else where the last main window was, or centred at the first size.
    pub(crate) bounds: Option<WindowBounds>,
    /// The display to centre it on, when it isn't placed.
    pub(crate) display: Option<DisplayId>,
    /// Open without taking the keyboard focus from the app in front.
    pub(crate) background: bool,
}

/// Starts keeping the windows over `store`; the first main window's report panel takes
/// `report_size`.
pub(crate) fn install(store: Entity<Store>, report_size: ReportSize, cx: &mut App) {
    cx.set_global(Windows {
        store,
        mains: Vec::new(),
        singles: HashMap::new(),
        views: HashMap::new(),
        first_report_size: Some(report_size),
    });
    cx.on_window_closed(|cx, id| closed(id, cx)).detach();
}

/// The windows' global, once [`install`]ed.
fn windows_mut(cx: &mut App) -> Option<&mut Windows> {
    cx.has_global::<Windows>()
        .then(|| cx.global_mut::<Windows>())
}

/// The store the windows show.
#[must_use]
pub(crate) fn store(cx: &App) -> Option<Entity<Store>> {
    cx.try_global::<Windows>()
        .map(|windows| windows.store.clone())
}

/// Registers the view a window of its own is made of (its feature's one line, after
/// [`install`]).
pub(crate) fn register_view(kind: WindowKind, view: ViewBuilder, cx: &mut App) {
    match windows_mut(cx) {
        Some(windows) => {
            windows.views.insert(kind, view);
        }
        None => log::warn!("the {} window was registered too early", kind.title()),
    }
}

/// The kinds with a window in this build, for the sidebar's window rows.
#[must_use]
pub(crate) fn available(cx: &App) -> Vec<WindowKind> {
    let Some(windows) = cx.try_global::<Windows>() else {
        return Vec::new();
    };
    WindowKind::ALL
        .into_iter()
        .filter(|kind| windows.views.contains_key(kind))
        .collect()
}

/// Opens a main window.
///
/// # Errors
///
/// When the platform can't open a window, or [`install`] hasn't run.
pub(crate) fn open_main(
    opening: Opening,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<MainWindow>)> {
    let store = store(cx).context("the windows aren't set up")?;
    let report_size = windows_mut(cx)
        .and_then(|windows| windows.first_report_size.take())
        .unwrap_or_default();
    let bounds = opening
        .bounds
        .unwrap_or_else(|| next_main_bounds(opening.display, cx));
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            ..TitlebarOptions::default()
        }),
        app_id: Some(APP_ID.into()),
        focus: !opening.background,
        display_id: opening.display,
        window_background: look::background_now(cx),
        ..WindowOptions::default()
    };
    let (handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
        let page = store.read(cx).theme().page().clone();
        look::apply(&page, window, cx);
        // An answer still in its Undo window isn't lost to the app going to the background,
        // which may be the last the user sees of it.
        Store::send_held_when_inactive(&store, window, cx);
        cx.new(|cx| MainWindow::new(store.clone(), report_size, window, cx))
    })
    .context("couldn't open the window")?;
    look::follow(handle, store, cx);
    if let Some(windows) = windows_mut(cx) {
        windows.mains.insert(0, (handle, view.downgrade()));
    }
    Ok((handle, view))
}

/// Where a new main window goes: a step down and right of the last one used, else where the
/// last one was left (kept across launches), else centred at the first size.
fn next_main_bounds(display: Option<DisplayId>, cx: &mut App) -> WindowBounds {
    if let Some(last) = front_main(cx)
        && let Ok(bounds) = last.update(cx, |_, window, _| window.bounds())
    {
        return WindowBounds::Windowed(Bounds::new(
            point(bounds.origin.x + px(CASCADE), bounds.origin.y + px(CASCADE)),
            bounds.size,
        ));
    }
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.bounds())
        .collect();
    Kept::get(cx)
        .frame
        .and_then(|frame| frame.restore(&displays))
        .unwrap_or_else(|| {
            let (width, height) = prefs::FIRST_SIZE;
            WindowBounds::Windowed(Bounds::centered(display, size(px(width), px(height)), cx))
        })
}

/// The main window used last, if one is open.
#[must_use]
pub(crate) fn front_main(cx: &App) -> Option<AnyWindowHandle> {
    let windows = cx.try_global::<Windows>()?;
    if let Some(active) = cx.active_window()
        && windows.mains.iter().any(|(handle, _)| *handle == active)
    {
        return Some(active);
    }
    windows.mains.first().map(|(handle, _)| *handle)
}

/// Runs `f` on the main window used last, brought to the front, opening one if none is open.
pub(crate) fn with_main(
    cx: &mut App,
    f: impl FnOnce(&mut MainWindow, &mut Window, &mut Context<'_, MainWindow>),
) {
    let handle = match front_main(cx) {
        Some(handle) => handle,
        None => match open_main(Opening::default(), cx) {
            Ok((handle, _)) => handle,
            Err(error) => {
                log::error!("{error:#}");
                return;
            }
        },
    };
    let Some(view) = cx.try_global::<Windows>().and_then(|windows| {
        windows
            .mains
            .iter()
            .find(|(open, _)| *open == handle)
            .and_then(|(_, view)| view.upgrade())
    }) else {
        return;
    };
    let updated = handle.update(cx, |_, window, cx| {
        window.activate_window();
        view.update(cx, |main, cx| f(main, window, cx));
    });
    if let Err(error) = updated {
        log::debug!("the main window closed before it could be used: {error}");
    }
    cx.activate(true);
}

/// A main window came to the front: it is the one links and menus go to.
pub(crate) fn note_front(handle: AnyWindowHandle, cx: &mut App) {
    if let Some(windows) = windows_mut(cx)
        && let Some(at) = windows.mains.iter().position(|(open, _)| *open == handle)
    {
        let main = windows.mains.remove(at);
        windows.mains.insert(0, main);
    }
}

/// The Dock icon clicked (or the app opened again) with no window: a main window opens; with
/// one, it comes forward.
pub(crate) fn reopen(cx: &mut App) {
    if front_main(cx).is_some() {
        with_main(cx, |_, _, _| {});
    } else if let Err(error) = open_main(Opening::default(), cx) {
        log::error!("{error:#}");
    }
}

/// Opens the window of its own `kind`, or brings it forward if it is open. `false` when this
/// build has no such window (nothing opens).
pub(crate) fn open_single(kind: WindowKind, cx: &mut App) -> bool {
    open_single_as(kind, true, cx)
}

/// Opens the window of its own `kind` as [`open_single`] does; without `focus`, behind the app
/// in front and without the keyboard (reopened at launch, before the main window).
pub(super) fn open_single_as(kind: WindowKind, focus: bool, cx: &mut App) -> bool {
    let Some(windows) = cx.try_global::<Windows>() else {
        return false;
    };
    let open = windows.singles.get(&kind).copied();
    let view = windows.views.get(&kind).cloned();
    let store = windows.store.clone();
    if let Some(handle) = open
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        cx.activate(true);
        return true;
    }
    let Some(view) = view else {
        return false;
    };
    let options = WindowOptions {
        window_bounds: Some(super::restore::bounds(kind, cx)),
        titlebar: Some(TitlebarOptions {
            title: Some(kind.title().into()),
            ..TitlebarOptions::default()
        }),
        app_id: Some(APP_ID.into()),
        focus,
        window_background: look::background_now(cx),
        ..WindowOptions::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let page = store.read(cx).theme().page().clone();
        look::apply(&page, window, cx);
        let content = view(store.clone(), window, cx);
        cx.new(|cx| Single::new(kind, content, &store, window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            if let Some(windows) = windows_mut(cx) {
                windows.singles.insert(kind, handle);
            }
            if focus {
                cx.activate(true);
            }
            true
        }
        Err(error) => {
            log::error!("couldn't open the {} window: {error:#}", kind.title());
            false
        }
    }
}

/// A window of its own's root: the view its feature made.
struct Single(AnyView);

impl Single {
    /// The root over `content`, a window of `kind`. The window counts for where the app is, as
    /// a main window does ([`super::presence::follow`]), and its frame is kept for the next
    /// time it opens.
    fn new(
        kind: WindowKind,
        content: AnyView,
        store: &Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        super::presence::follow(window, store, cx);
        cx.observe_window_bounds(window, move |_, window, cx| {
            super::restore::note_frame(kind, window.window_bounds(), cx);
        })
        .detach();
        Self(content)
    }
}

impl Render for Single {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // Locked, it shows the lock's cover instead.
        match crate::ui::lock::cover(&crate::ui::palette::pal(cx), cx) {
            Some(cover) => cover.into_any_element(),
            None => self.0.clone().into_any_element(),
        }
    }
}

/// A window closed: it is forgotten (where the app is, too); a main window's column goes and
/// its frame is written to disk.
fn closed(id: WindowId, cx: &mut App) {
    let Some(windows) = windows_mut(cx) else {
        return;
    };
    let was_main = windows
        .mains
        .iter()
        .any(|(handle, _)| handle.window_id() == id);
    let was_single = windows
        .singles
        .values()
        .any(|handle| handle.window_id() == id);
    windows.mains.retain(|(handle, _)| handle.window_id() != id);
    windows.singles.retain(|_, handle| handle.window_id() != id);
    let store = windows.store.clone();
    super::presence::closed(id, &store, cx);
    if was_main {
        crate::ui::column::closed(id, &store, cx);
    }
    // Its frame, for the next launch.
    if was_main || was_single {
        Kept::save(cx);
    }
}

/// The windows of their own that are open.
#[must_use]
pub(super) fn open_singles(cx: &App) -> Vec<WindowKind> {
    cx.try_global::<Windows>().map_or_else(Vec::new, |windows| {
        WindowKind::ALL
            .into_iter()
            .filter(|kind| windows.singles.contains_key(kind))
            .collect()
    })
}

/// Whether `handle` is one of the main windows.
#[must_use]
pub(crate) fn is_main(handle: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<Windows>()
        .is_some_and(|windows| windows.mains.iter().any(|(open, _)| *open == handle))
}

/// How many main windows are open.
#[cfg(test)]
#[must_use]
pub(crate) fn main_count(cx: &App) -> usize {
    cx.try_global::<Windows>()
        .map_or(0, |windows| windows.mains.len())
}
