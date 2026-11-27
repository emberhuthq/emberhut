//! How the window was left, kept across launches in `view.json`: the open room, whether the
//! sidebar's Rooms section is folded, the sidebar's width once its edge was dragged, and
//! whether the sidebar shows the agents and where. Read
//! before the run loop starts, as [`json_file`] reads; written off the UI thread whenever the
//! room or the fold changes, and when a drag of the sidebar's edge ends (not on each step of
//! it). A width read back is kept inside [`SIDEBAR_WIDTHS`].
//!
//! The report panel's width is kept apart, in `layout.json` (the window's, [`AppFiles::layout`]):
//! each file has one writer, so neither write can lose the other's keys.

use emberhut_api::ids::RoomId;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use serde::{Deserialize, Serialize};

use super::Store;
use crate::config::AppFiles;
use crate::json_file;

/// What `view.json` holds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ViewFile {
    /// The room on screen when the app was last used.
    pub(crate) room: Option<RoomId>,
    /// Whether the sidebar's Rooms section is open.
    pub(crate) rooms_open: bool,
    /// The sidebar's width, in logical pixels, once the user dragged its edge.
    pub(crate) sidebar_width: Option<f32>,
    /// Whether the sidebar lists the agents. Hidden, they stay in Settings › Appearance and in
    /// Find.
    pub(crate) agents_shown: bool,
    /// Where the sidebar lists the agents.
    pub(crate) agents_place: AgentsPlace,
}

/// Where the sidebar lists the agents. A file written before the choice existed, or naming a
/// place this build doesn't know, reads as [`AgentsPlace::Below`]: the rooms are what the
/// sidebar is for, and the agents are reached less often.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AgentsPlace {
    /// Right under Needs you, Tasks and Scheduled, above the rooms.
    Above,
    /// After the rooms, the projects, the sessions and the goals, just above More.
    #[default]
    #[serde(other)]
    Below,
}

impl Default for ViewFile {
    fn default() -> Self {
        Self {
            room: None,
            rooms_open: true,
            sidebar_width: None,
            agents_shown: true,
            agents_place: AgentsPlace::Below,
        }
    }
}

/// The folds open for the session only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Folds {
    /// The sidebar's Paused fold (it starts folded).
    paused: bool,
    /// Settings' list of archived rooms.
    archived: bool,
}

/// The sidebar's width until the user drags its edge.
pub(crate) const SIDEBAR_WIDTH: f32 = 260.;
/// How narrow and how wide the sidebar can be dragged.
pub(crate) const SIDEBAR_WIDTHS: (f32, f32) = (190., 420.);

/// A dragged (or kept) width inside [`SIDEBAR_WIDTHS`], in whole pixels.
#[must_use]
pub(crate) fn sidebar_width_for(dragged: f32) -> f32 {
    dragged.round().clamp(SIDEBAR_WIDTHS.0, SIDEBAR_WIDTHS.1)
}

impl ViewFile {
    /// What `files` keep; the default when nothing is kept or it doesn't read. Reads a file:
    /// call it before the app's run loop starts.
    #[must_use]
    pub(crate) fn read(files: Option<&AppFiles>) -> Self {
        let read = files.and_then(|files| {
            json_file::read(&files.view(), "a view", "the window opens as it first does")
        });
        read.map_or_default(|view: Self| Self {
            sidebar_width: view.sidebar_width.map(sidebar_width_for),
            ..view
        })
    }
}

impl Store {
    /// Whether the sidebar's Rooms section is open.
    #[must_use]
    pub(crate) fn rooms_open(&self) -> bool {
        self.view.rooms_open
    }

    /// Folds the sidebar's Rooms section, or opens it again; kept across launches.
    pub(crate) fn toggle_rooms(&mut self, cx: &mut Context<'_, Self>) {
        self.view.rooms_open = !self.view.rooms_open;
        self.keep_view(cx);
        cx.notify();
    }

    /// Whether the sidebar lists the agents.
    #[must_use]
    pub(crate) fn agents_shown(&self) -> bool {
        self.view.agents_shown
    }

    /// Shows the agents in the sidebar, or hides them; kept across launches.
    pub(crate) fn show_agents(&mut self, shown: bool, cx: &mut Context<'_, Self>) {
        if self.view.agents_shown != shown {
            self.view.agents_shown = shown;
            self.keep_view(cx);
            cx.notify();
        }
    }

    /// Where the sidebar lists the agents.
    #[must_use]
    pub(crate) fn agents_place(&self) -> AgentsPlace {
        self.view.agents_place
    }

    /// Lists the agents `place` in the sidebar; kept across launches.
    pub(crate) fn place_agents(&mut self, place: AgentsPlace, cx: &mut Context<'_, Self>) {
        if self.view.agents_place != place {
            self.view.agents_place = place;
            self.keep_view(cx);
            cx.notify();
        }
    }

    /// The sidebar's width.
    #[must_use]
    pub(crate) fn sidebar_width(&self) -> f32 {
        self.view.sidebar_width.unwrap_or(SIDEBAR_WIDTH)
    }

    /// The user is dragging the sidebar's edge, now at `dragged`: its width from now on, kept
    /// once the drag ends ([`Self::let_go_of_sidebar`]).
    pub(crate) fn drag_sidebar_to(&mut self, dragged: f32, cx: &mut Context<'_, Self>) {
        let width = sidebar_width_for(dragged);
        if self.view.sidebar_width != Some(width) {
            self.view.sidebar_width = Some(width);
            cx.notify();
        }
    }

    /// The user let go of the pointer: a width the sidebar was dragged to is kept across
    /// launches.
    pub(crate) fn let_go_of_sidebar(&mut self, cx: &mut Context<'_, Self>) {
        if self.sidebar_kept != self.view.sidebar_width {
            self.sidebar_kept = self.view.sidebar_width;
            self.keep_view(cx);
        }
    }

    /// Whether the sidebar's Paused fold is open: it starts folded each launch.
    #[must_use]
    pub(crate) fn paused_open(&self) -> bool {
        self.folds.paused
    }

    /// Opens the sidebar's Paused fold, or folds it again.
    pub(crate) fn toggle_paused(&mut self, cx: &mut Context<'_, Self>) {
        self.folds.paused = !self.folds.paused;
        cx.notify();
    }

    /// Whether Settings' list of archived rooms is open.
    #[must_use]
    pub(crate) fn archived_open(&self) -> bool {
        self.folds.archived
    }

    /// Opens Settings' list of archived rooms, or folds it.
    pub(crate) fn toggle_archived(&mut self, cx: &mut Context<'_, Self>) {
        self.folds.archived = !self.folds.archived;
        cx.notify();
    }

    /// Starts from how the window was left (the open room is the command line's or this).
    pub(crate) fn start_view(&mut self, view: ViewFile) {
        self.sidebar_kept = view.sidebar_width;
        self.view = ViewFile {
            room: Some(self.room),
            ..view
        };
    }

    /// The open room changed: kept for the next launch.
    pub(super) fn room_opened(&mut self, cx: &mut Context<'_, Self>) {
        if self.view.room != Some(self.room) {
            self.view.room = Some(self.room);
            self.keep_view(cx);
        }
    }

    /// Writes the view off the UI thread; the newest write is the one that stays.
    fn keep_view(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = self.files.as_ref().map(AppFiles::view) else {
            return;
        };
        let (number, written) = self.view_writes.next();
        let view = self.view;
        cx.background_spawn(async move {
            json_file::write(&path, &view, number, &written, "the view");
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gpui_kit::{Entity, TestAppContext};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::store::ThemeState;

    fn write(files: &AppFiles, view: &ViewFile) {
        json_file::write(&files.view(), view, 1, &Mutex::new(0), "the view");
    }

    #[test]
    fn what_is_kept_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        assert_eq!(
            ViewFile::read(Some(&files)),
            ViewFile::default(),
            "nothing kept yet"
        );
        let view = ViewFile {
            room: Some(RoomId::new(4)),
            rooms_open: false,
            sidebar_width: Some(300.),
            agents_shown: false,
            agents_place: AgentsPlace::Above,
        };
        write(&files, &view);
        assert_eq!(ViewFile::read(Some(&files)), view);
    }

    #[test]
    fn a_broken_or_older_file_reads_as_far_as_it_goes() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        std::fs::write(files.view(), "{").unwrap();
        assert_eq!(ViewFile::read(Some(&files)), ViewFile::default());
        std::fs::write(files.view(), r#"{"room": 3}"#).unwrap();
        assert_eq!(
            ViewFile::read(Some(&files)),
            ViewFile {
                room: Some(RoomId::new(3)),
                rooms_open: true,
                sidebar_width: None,
                agents_shown: true,
                agents_place: AgentsPlace::Below,
            },
            "a file from before the agents' choice lists them, below the rooms"
        );
        std::fs::write(
            files.view(),
            r#"{"room": 3, "agentsShown": false, "agentsPlace": "sideways"}"#,
        )
        .unwrap();
        assert_eq!(
            ViewFile::read(Some(&files)),
            ViewFile {
                room: Some(RoomId::new(3)),
                agents_shown: false,
                ..ViewFile::default()
            },
            "a place this build doesn't know reads as below, and the rest is kept"
        );
    }

    #[test]
    fn a_kept_width_out_of_bounds_reads_inside_them() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        for (kept, read) in [(5., SIDEBAR_WIDTHS.0), (100_000., SIDEBAR_WIDTHS.1)] {
            std::fs::write(files.view(), format!(r#"{{"sidebarWidth": {kept}}}"#)).unwrap();
            assert_eq!(ViewFile::read(Some(&files)).sidebar_width, Some(read));
        }
    }

    fn store(cx: &mut TestAppContext, files: &AppFiles) -> Entity<Store> {
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| {
            Store::idle(
                backend,
                Some(files.clone()),
                ThemeState::load(None),
                RoomId::MAIN,
            )
        })
    }

    #[gpui_kit::test]
    fn the_open_room_and_the_folds_are_kept_for_the_next_launch(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let store = store(cx, &files);
        store.update(cx, |store, cx| {
            store.start_view(ViewFile::read(Some(&files)));
            store.switch_room(RoomId::new(4), cx);
            store.toggle_rooms(cx);
            store.drag_sidebar_to(301., cx);
            store.let_go_of_sidebar(cx);
            store.show_agents(false, cx);
            store.place_agents(AgentsPlace::Above, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            ViewFile::read(Some(&files)),
            ViewFile {
                room: Some(RoomId::new(4)),
                rooms_open: false,
                sidebar_width: Some(301.),
                agents_shown: false,
                agents_place: AgentsPlace::Above,
            }
        );
    }

    #[gpui_kit::test]
    fn a_drag_of_the_sidebar_is_kept_when_it_ends(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let store = store(cx, &files);
        store.update(cx, |store, cx| {
            for dragged in 280_i16..300 {
                store.drag_sidebar_to(f32::from(dragged), cx);
            }
            assert!((store.sidebar_width() - 299.).abs() < f32::EPSILON);
        });
        cx.run_until_parked();
        assert!(!files.view().exists(), "nothing written while dragging");
        store.update(cx, Store::let_go_of_sidebar);
        cx.run_until_parked();
        assert_eq!(ViewFile::read(Some(&files)).sidebar_width, Some(299.));
        std::fs::remove_file(files.view()).unwrap();
        store.update(cx, Store::let_go_of_sidebar);
        cx.run_until_parked();
        assert!(!files.view().exists(), "a click elsewhere writes nothing");
    }

    #[test]
    fn the_sidebar_drags_between_its_bounds() {
        assert!((sidebar_width_for(300.4) - 300.).abs() < f32::EPSILON);
        assert!((sidebar_width_for(20.) - SIDEBAR_WIDTHS.0).abs() < f32::EPSILON);
        assert!((sidebar_width_for(2000.) - SIDEBAR_WIDTHS.1).abs() < f32::EPSILON);
    }
}
