//! How the user left the app's windows, kept across launches in `window.json` in the app's
//! directory: the main window's frame (its size and place, or that it filled the screen), the
//! sidebar's More, Recent sessions, Projects and Goals folds (its width and the Rooms fold are
//! the store's, in `view.json`), and each window of its own's frame and whether it was open.
//! A missing or unreadable file is the defaults: the window
//! opens at its first-launch size, centred.
//!
//! The file is read before the run loop starts (no file is read on the UI thread). Every write
//! goes through [`json_file`] (a temporary file, then a rename) and is numbered, so a newer
//! write always wins and two can't interleave. The folds are written as they change, on a
//! background thread. A file window's size, which changes with every pixel of a drag, is
//! written once it has stayed put for [`SETTLE`]. The frame changes with every pixel of a drag
//! too, so it is kept in memory and written when a main window closes and when the app quits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::{App, Bounds, Global, Pixels, Task, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::config::AppFiles;
use crate::json_file::{self, Writes};

/// The file's name in the app's directory.
const FILE: &str = "window.json";

/// How long a file window's size stays put before it is written.
const SETTLE: Duration = Duration::from_millis(600);

/// The main window's size the first time (as the reference client opens it).
pub(crate) const FIRST_SIZE: (f32, f32) = (1100., 760.);

/// The smallest kept frame a main window opens at (a smaller one is taken as damaged).
const SMALLEST_MAIN: (f32, f32) = (400., 300.);

/// What is kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Prefs {
    /// The main window's last frame.
    pub(crate) frame: Option<Frame>,
    /// The sidebar.
    pub(crate) sidebar: Sidebar,
    /// The size the user last gave a file's own window (its width and height).
    pub(crate) file_window: Option<(f32, f32)>,
    /// The width the user gave the column beside the chat by dragging its edge (none: the
    /// standard one).
    pub(crate) column_width: Option<f32>,
    /// What the Calendar window shows: a day, a week or a month.
    pub(crate) calendar_mode: crate::store::calendar::range::Mode,
    /// The windows of their own (Calendar, Sessions, …) as the user left them.
    pub(crate) own_windows: OwnWindows,
}

/// The windows of their own as the user left them, each by the name links use for it
/// (`calendar`, `sessions`, …), so a name this build doesn't know is skipped, not an error.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct OwnWindows {
    /// Each one's last frame.
    pub(crate) frames: BTreeMap<String, Frame>,
    /// The ones open when the app last quit.
    pub(crate) open: Vec<String>,
}

/// A window's frame, in logical pixels on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Frame {
    /// Its left edge.
    pub(crate) x: f32,
    /// Its top edge.
    pub(crate) y: f32,
    /// Its width (when not maximised or full screen: the size it returns to).
    pub(crate) width: f32,
    /// Its height.
    pub(crate) height: f32,
    /// How it was shown.
    #[serde(default)]
    pub(crate) state: FrameState,
}

/// How a window was shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrameState {
    /// In a window of its frame's size.
    #[default]
    Windowed,
    /// Maximised (zoomed).
    Maximized,
    /// Full screen.
    Fullscreen,
}

/// The sidebar's folds as the user left them.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each of the sidebar's folds is kept on its own"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Sidebar {
    /// Whether More is unfolded (folded at first).
    pub(crate) more_open: bool,
    /// Whether Recent sessions is unfolded (unfolded at first).
    pub(crate) sessions_open: bool,
    /// Whether Projects is unfolded (unfolded at first).
    pub(crate) projects_open: bool,
    /// Whether Goals is unfolded (unfolded at first).
    pub(crate) goals_open: bool,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            more_open: false,
            sessions_open: true,
            projects_open: true,
            goals_open: true,
        }
    }
}

impl Frame {
    /// The frame of a window in `bounds`.
    #[must_use]
    pub(crate) fn of(bounds: WindowBounds) -> Self {
        let (state, at) = match bounds {
            WindowBounds::Windowed(at) => (FrameState::Windowed, at),
            WindowBounds::Maximized(at) => (FrameState::Maximized, at),
            WindowBounds::Fullscreen(at) => (FrameState::Fullscreen, at),
        };
        Self {
            x: f32::from(at.origin.x),
            y: f32::from(at.origin.y),
            width: f32::from(at.size.width),
            height: f32::from(at.size.height),
            state,
        }
    }

    /// The frame's place and size.
    #[must_use]
    pub(crate) fn bounds(self) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.x), px(self.y)),
            size(px(self.width), px(self.height)),
        )
    }

    /// The window bounds a main window opens with, or `None` when the frame isn't usable: a
    /// size too small to work in, or a place on none of `displays` (one unplugged since).
    #[must_use]
    pub(crate) fn restore(self, displays: &[Bounds<Pixels>]) -> Option<WindowBounds> {
        self.restore_at_least(displays, SMALLEST_MAIN)
    }

    /// The window bounds to open with, or `None` when the frame isn't usable: smaller than
    /// `smallest` (its width and height), or on none of `displays`.
    #[must_use]
    pub(crate) fn restore_at_least(
        self,
        displays: &[Bounds<Pixels>],
        smallest: (f32, f32),
    ) -> Option<WindowBounds> {
        let usable = [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            && self.width >= smallest.0
            && self.height >= smallest.1;
        let bounds = self.bounds();
        // Its title bar must be on a display, to be dragged back.
        let grip = point(bounds.center().x, bounds.origin.y + px(10.));
        if !usable || !displays.iter().any(|display| display.contains(&grip)) {
            return None;
        }
        Some(match self.state {
            FrameState::Windowed => WindowBounds::Windowed(bounds),
            FrameState::Maximized => WindowBounds::Maximized(bounds),
            FrameState::Fullscreen => WindowBounds::Fullscreen(bounds),
        })
    }
}

impl Prefs {
    /// Reads `window.json` in `dir`; missing or unreadable, the defaults (logged when it is
    /// there but not readable).
    #[must_use]
    pub(crate) fn read(dir: &Path) -> Self {
        let path = path(dir);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!("{} isn't valid ({error}); starting afresh", path.display());
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("can't read {} ({error}); starting afresh", path.display());
                Self::default()
            }
        }
    }
}

/// `window.json` in `dir`.
fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The prefs while the app runs, and where they are kept.
#[derive(Debug)]
pub(crate) struct Kept {
    prefs: Prefs,
    files: Option<AppFiles>,
    /// The numbers of the writes of `window.json`, so the newest is the one kept.
    writes: Writes,
    /// The column's width was dragged since `window.json` was last written.
    column_moved: bool,
    /// The write waiting for a drag to settle; replacing it drops (cancels) the one before.
    settling: Option<Task<()>>,
}

impl Global for Kept {}

impl Kept {
    /// The prefs read from `files` (the defaults without a directory).
    #[must_use]
    pub(crate) fn load(files: Option<AppFiles>) -> Self {
        let prefs = files
            .as_ref()
            .map_or_else(Prefs::default, |files| Prefs::read(files.dir()));
        Self {
            prefs,
            files,
            writes: Writes::default(),
            column_moved: false,
            settling: None,
        }
    }

    /// The prefs now (the defaults before [`Kept::load`] is installed, as in the views' tests).
    #[must_use]
    pub(crate) fn get(cx: &App) -> Prefs {
        cx.try_global::<Self>()
            .map_or_else(Prefs::default, |kept| kept.prefs.clone())
    }

    /// Keeps what the Calendar window shows, written on a background thread if it changed.
    pub(crate) fn set_calendar_mode(cx: &mut App, mode: crate::store::calendar::range::Mode) {
        let Some(kept) = cx.try_global::<Self>() else {
            return;
        };
        if kept.prefs.calendar_mode == mode {
            return;
        }
        cx.global_mut::<Self>().prefs.calendar_mode = mode;
        Self::save(cx);
    }

    /// Changes the sidebar's prefs and writes them, on a background thread, if they changed.
    pub(crate) fn set_sidebar(cx: &mut App, change: impl FnOnce(&mut Sidebar)) {
        let Some(kept) = cx.try_global::<Self>() else {
            return;
        };
        let mut sidebar = kept.prefs.sidebar;
        change(&mut sidebar);
        if sidebar == kept.prefs.sidebar {
            return;
        }
        cx.global_mut::<Self>().prefs.sidebar = sidebar;
        Self::save(cx);
    }

    /// Notes the main window's frame (kept in memory: see [`Kept::save`]).
    pub(crate) fn note_frame(cx: &mut App, bounds: WindowBounds) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().prefs.frame = Some(Frame::of(bounds));
        }
    }

    /// Notes the frame of the window of its own called `name` (kept in memory: see
    /// [`Kept::save`]).
    pub(crate) fn note_own_window(cx: &mut App, name: &str, bounds: WindowBounds) {
        if cx.has_global::<Self>() {
            let frames = &mut cx.global_mut::<Self>().prefs.own_windows.frames;
            frames.insert(name.to_owned(), Frame::of(bounds));
        }
    }

    /// Notes which windows of their own are open, by name (kept in memory, for the quit).
    pub(crate) fn note_open_windows(cx: &mut App, names: Vec<String>) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().prefs.own_windows.open = names;
        }
    }

    /// Notes the size of a file's own window, written once it settles ([`SETTLE`]).
    pub(crate) fn note_file_window(cx: &mut App, width: f32, height: f32) {
        let Some(kept) = cx.try_global::<Self>() else {
            return;
        };
        let size = Some((width, height));
        if !width.is_finite() || !height.is_finite() || kept.prefs.file_window == size {
            return;
        }
        cx.global_mut::<Self>().prefs.file_window = size;
        Self::save_settled(cx);
    }

    /// Writes the prefs off the UI thread once nothing has changed them for [`SETTLE`]: a
    /// change before then puts the write off again.
    fn save_settled(cx: &mut App) {
        let settling = cx.spawn(async |cx| {
            cx.background_executor().timer(SETTLE).await;
            cx.update(Self::save);
        });
        cx.global_mut::<Self>().settling = Some(settling);
    }

    /// Notes the width the column's edge was dragged to (`None`: the standard one again), kept
    /// in memory until [`Kept::save_column_width`]; a width that isn't a number isn't kept.
    pub(crate) fn note_column_width(cx: &mut App, width: Option<f32>) {
        if cx.has_global::<Self>() {
            let kept = cx.global_mut::<Self>();
            kept.prefs.column_width = width.filter(|width| width.is_finite());
            kept.column_moved = true;
        }
    }

    /// Writes the prefs off the UI thread if the column's width was dragged since they were
    /// last written (the pointer let go of its edge).
    pub(crate) fn save_column_width(cx: &mut App) {
        if cx
            .try_global::<Self>()
            .is_some_and(|kept| kept.column_moved)
        {
            Self::save(cx);
        }
    }

    /// Writes the prefs off the UI thread (the sidebar's folds changed, a main window closed).
    pub(crate) fn save(cx: &mut App) {
        if let Some((path, prefs, number, written)) = Self::next_write(cx) {
            cx.background_spawn(async move {
                json_file::write(&path, &prefs, number, &written, "the window's frame");
            })
            .detach();
        }
    }

    /// Writes the prefs now, on this thread, as the app quits (nothing runs after); it still
    /// takes its turn after the writes before it.
    pub(crate) fn save_on_quit(cx: &mut App) {
        if let Some((path, prefs, number, written)) = Self::next_write(cx) {
            json_file::write(&path, &prefs, number, &written, "the window's frame");
        }
    }

    /// What the next write writes, and where: none without a directory.
    fn next_write(cx: &mut App) -> Option<(PathBuf, Prefs, u64, Arc<Mutex<u64>>)> {
        if !cx.has_global::<Self>() {
            return None;
        }
        let kept = cx.global_mut::<Self>();
        let path = path(kept.files.as_ref()?.dir());
        kept.column_moved = false;
        let (number, written) = kept.writes.next();
        Some((path, kept.prefs.clone(), number, written))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(1920.), px(1080.)))
    }

    fn frame(x: f32, y: f32, width: f32, height: f32) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
            state: FrameState::Windowed,
        }
    }

    #[test]
    fn a_frame_on_a_display_is_restored_as_it_was() {
        let kept = frame(100., 80., 1100., 760.);
        assert_eq!(
            kept.restore(&[display()]),
            Some(WindowBounds::Windowed(kept.bounds()))
        );
        let zoomed = Frame {
            state: FrameState::Maximized,
            ..kept
        };
        assert_eq!(
            zoomed.restore(&[display()]),
            Some(WindowBounds::Maximized(kept.bounds()))
        );
        assert_eq!(
            Frame::of(WindowBounds::Fullscreen(kept.bounds())).state,
            FrameState::Fullscreen
        );
    }

    #[test]
    fn a_frame_off_every_display_or_too_small_is_not() {
        assert_eq!(frame(3000., 80., 1100., 760.).restore(&[display()]), None);
        assert_eq!(frame(100., 80., 1100., 760.).restore(&[]), None);
        assert_eq!(frame(100., 80., 120., 90.).restore(&[display()]), None);
        assert_eq!(
            frame(f32::NAN, 80., 1100., 760.).restore(&[display()]),
            None
        );
        let note = frame(100., 80., 460., 280.);
        assert_eq!(
            note.restore(&[display()]),
            None,
            "too small for a main window"
        );
        assert_eq!(
            note.restore_at_least(&[display()], (200., 150.)),
            Some(WindowBounds::Windowed(note.bounds())),
            "a quick note's size is fine for its own window"
        );
    }

    #[test]
    fn the_sidebar_starts_folded_where_the_reference_client_does() {
        let sidebar = Sidebar::default();
        assert!(!sidebar.more_open, "More starts folded");
        assert!(sidebar.sessions_open, "Recent sessions starts open");
        assert!(
            sidebar.projects_open && sidebar.goals_open,
            "Projects and Goals too"
        );
    }

    #[test]
    fn prefs_round_trip_through_their_file_and_a_bad_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Prefs::read(dir.path()), Prefs::default(), "no file yet");
        let prefs = Prefs {
            frame: Some(frame(10., 20., 900., 700.)),
            sidebar: Sidebar {
                more_open: true,
                sessions_open: false,
                projects_open: false,
                goals_open: true,
            },
            file_window: Some((900., 640.)),
            column_width: Some(500.),
            calendar_mode: crate::store::calendar::range::Mode::Month,
            own_windows: OwnWindows {
                frames: BTreeMap::from([("calendar".to_owned(), frame(40., 60., 1180., 820.))]),
                open: vec!["calendar".to_owned()],
            },
        };
        json_file::write(&path(dir.path()), &prefs, 1, &Mutex::new(0), "the prefs");
        assert_eq!(Prefs::read(dir.path()), prefs);
        std::fs::write(dir.path().join(FILE), b"{ not json").unwrap();
        assert_eq!(Prefs::read(dir.path()), Prefs::default());
        std::fs::write(dir.path().join(FILE), br#"{"sidebar":{"more_open":true}}"#).unwrap();
        let partial = Prefs::read(dir.path());
        assert!(partial.sidebar.more_open);
        assert!(
            partial.sidebar.sessions_open,
            "the fold not written is its default"
        );
    }

    #[gpui_kit::test]
    fn a_file_windows_size_is_written_once_it_settles(cx: &mut gpui_kit::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| cx.set_global(Kept::load(Some(AppFiles::in_dir(dir.path())))));
        cx.update(|cx| Kept::note_file_window(cx, 800., 600.));
        cx.executor().advance_clock(SETTLE / 2);
        cx.update(|cx| Kept::note_file_window(cx, 900., 640.));
        cx.executor().advance_clock(SETTLE / 2);
        cx.run_until_parked();
        assert_eq!(
            Prefs::read(dir.path()),
            Prefs::default(),
            "not while it moves"
        );
        cx.executor().advance_clock(SETTLE);
        cx.run_until_parked();
        assert_eq!(Prefs::read(dir.path()).file_window, Some((900., 640.)));
    }
}
