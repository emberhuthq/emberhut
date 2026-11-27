//! How the user looks at Needs you: as a list or a board, grouped by project or by when the asks
//! have to be decided, and narrowed by project, area, owner and (on the list) when. The picks
//! are kept on this device (`needs.json` in the app's directory), so the page opens as it was
//! left (with a project page's list or board). Which board columns are folded holds for the
//! session: Later starts folded.

use std::path::{Path, PathBuf};

use emberhut_api::ids::{AreaId, ProjectId};
use emberhut_api::state::State;
use emberhut_api::work::WorkRow;
use gpui_kit::{AppContext as _, Context};
use serde::{Deserialize, Serialize};

use super::Store;
use super::lookup;
use super::needs::Needs;
use super::tasks::filter::{OwnerPick, ProjectPick, When};
use crate::config::AppFiles;

/// List or board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Layout {
    /// One column of cards, in sections.
    List,
    /// A column per group, side by side: the default on a desktop.
    #[default]
    Board,
}

/// What the asks are grouped by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Grouping {
    /// A group per project, the one with the most urgent ask first, Other last.
    #[default]
    Project,
    /// Today, This week, This month.
    When,
}

/// The pick on Needs you; each part left out narrows nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NeedsFilter {
    /// By project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<ProjectPick>,
    /// By a project's area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) area: Option<AreaId>,
    /// By whose task it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<OwnerPick>,
    /// By when it has to be decided (the list only: the board's columns may be just that).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) when: Option<When>,
}

impl NeedsFilter {
    /// How many parts are on (the bar offers Clear from two).
    #[must_use]
    pub(crate) fn count(&self) -> usize {
        [
            self.project.is_some(),
            self.area.is_some(),
            self.owner.is_some(),
            self.when.is_some(),
        ]
        .into_iter()
        .filter(|&on| on)
        .count()
    }

    /// The filter as it applies: with projects hidden a project or area picked before no longer
    /// narrows; on the board, when doesn't either.
    #[must_use]
    pub(crate) fn shown(&self, hide_projects: bool, layout: Layout) -> Self {
        Self {
            project: self.project.filter(|_| !hide_projects),
            area: self.area.filter(|_| !hide_projects),
            owner: self.owner,
            when: self.when.filter(|_| layout == Layout::List),
        }
    }

    /// Whether it keeps `row`, an ask in `needs`.
    #[must_use]
    pub(crate) fn keeps(&self, row: &WorkRow, needs: &Needs<'_>, state: &State) -> bool {
        if let Some(project) = self.project {
            let matches = match project {
                ProjectPick::NoProject => row.pid.is_none(),
                ProjectPick::Project(id) => row.pid == Some(id),
            };
            if !matches {
                return false;
            }
        }
        if self.area.is_some_and(|area| row.area != Some(area)) {
            return false;
        }
        if let Some(owner) = self.owner {
            let task = row.task.and_then(|task| lookup::issue(state, task));
            if task
                .is_none_or(|task| lookup::is_assistants(task) != (owner == OwnerPick::Assistants))
            {
                return false;
            }
        }
        self.when.is_none_or(|when| needs.when(row) == when)
    }
}

/// The projects the asks are in, each once, in the asks' order: only these are offered, so a
/// pick never comes up empty (`None` is No project).
#[must_use]
pub(crate) fn projects_of(rows: &[&WorkRow]) -> Vec<Option<ProjectId>> {
    let mut seen = Vec::new();
    for row in rows {
        if !seen.contains(&row.pid) {
            seen.push(row.pid);
        }
    }
    seen
}

/// The areas the asks are in, in the state's order.
#[must_use]
pub(crate) fn areas_of(rows: &[&WorkRow], state: &State) -> Vec<AreaId> {
    state
        .areas
        .iter()
        .map(|area| area.id)
        .filter(|&id| rows.iter().any(|row| row.area == Some(id)))
        .collect()
}

/// A board column's fold key: a project's, Other's, a time's or Later's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ColumnKey {
    /// A project's column (`None`: Other).
    Project(Option<ProjectId>),
    /// A time's column.
    When(When),
}

impl ColumnKey {
    /// Whether the column starts folded: Later does, the rest are open.
    #[must_use]
    pub(crate) fn starts_folded(&self) -> bool {
        *self == Self::When(When::Later)
    }
}

/// The picks kept on the device, and the folds of this session.
#[derive(Debug, Default)]
pub(crate) struct NeedsView {
    kept: Kept,
    /// The columns the user turned the other way from how they start.
    flipped: Vec<ColumnKey>,
}

/// What `needs.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    layout: Layout,
    #[serde(default)]
    grouping: Grouping,
    #[serde(default)]
    filter: NeedsFilter,
    /// A project's page: list or board.
    #[serde(default)]
    project_layout: Layout,
}

impl NeedsView {
    /// The picks kept in `files`, if any.
    #[must_use]
    pub(crate) fn load(files: Option<&AppFiles>) -> Self {
        Self {
            kept: files.map_or_else(Kept::default, |files| read(&path(files))),
            flipped: Vec::new(),
        }
    }

    /// List or board.
    #[must_use]
    pub(crate) fn layout(&self) -> Layout {
        self.kept.layout
    }

    /// By project or by when.
    #[must_use]
    pub(crate) fn grouping(&self) -> Grouping {
        self.kept.grouping
    }

    /// A project's page: list or board.
    #[must_use]
    pub(crate) fn project_layout(&self) -> Layout {
        self.kept.project_layout
    }

    /// The filter as picked (see [`NeedsFilter::shown`] for how it applies).
    #[must_use]
    pub(crate) fn filter(&self) -> &NeedsFilter {
        &self.kept.filter
    }

    /// Whether the board's column `key` is folded.
    #[must_use]
    pub(crate) fn is_folded(&self, key: &ColumnKey) -> bool {
        key.starts_folded() != self.flipped.contains(key)
    }

    fn flip(&mut self, key: ColumnKey) {
        match self.flipped.iter().position(|flipped| *flipped == key) {
            Some(at) => {
                self.flipped.swap_remove(at);
            }
            None => self.flipped.push(key),
        }
    }
}

/// Where the picks are kept.
fn path(files: &AppFiles) -> PathBuf {
    files.dir().join("needs.json")
}

/// The picks in `path`: none when there is no file, or it doesn't read (then it is logged, and
/// the next pick writes a good one).
fn read(path: &Path) -> Kept {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .inspect_err(|error| log::warn!("can't read {}: {error}", path.display()))
            .unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Kept::default(),
        Err(error) => {
            log::warn!("can't read {}: {error}", path.display());
            Kept::default()
        }
    }
}

fn write(path: &Path, kept: &Kept) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(kept)?)
}

impl Store {
    /// How the user looks at Needs you.
    #[must_use]
    pub(crate) fn needs_view(&self) -> &NeedsView {
        &self.needs_view
    }

    /// Shows Needs you as a list or a board; kept on this device.
    pub(crate) fn set_needs_layout(&mut self, layout: Layout, cx: &mut Context<'_, Self>) {
        self.needs_view.kept.layout = layout;
        self.keep_needs_view(cx);
    }

    /// Shows a project's page as a list or a board; kept on this device.
    pub(crate) fn set_project_layout(&mut self, layout: Layout, cx: &mut Context<'_, Self>) {
        self.needs_view.kept.project_layout = layout;
        self.keep_needs_view(cx);
    }

    /// Groups Needs you by project or by when; kept on this device.
    pub(crate) fn set_needs_grouping(&mut self, grouping: Grouping, cx: &mut Context<'_, Self>) {
        self.needs_view.kept.grouping = grouping;
        self.keep_needs_view(cx);
    }

    /// Narrows Needs you; kept on this device.
    pub(crate) fn set_needs_filter(&mut self, filter: NeedsFilter, cx: &mut Context<'_, Self>) {
        self.needs_view.kept.filter = filter;
        self.keep_needs_view(cx);
    }

    /// Folds the board's column `key`, or opens it.
    pub(crate) fn toggle_needs_column(&mut self, key: ColumnKey, cx: &mut Context<'_, Self>) {
        self.needs_view.flip(key);
        cx.notify();
    }

    /// Fetches the pictures the cards on Needs you show (each ask's newest picture) that
    /// aren't here yet.
    pub(super) fn fetch_needs_pictures(&mut self, cx: &mut Context<'_, Self>) {
        let paths: Vec<String> = self.state.as_ref().map_or_default(|state| {
            state
                .working
                .iter()
                .filter(|row| row.needs_you())
                .filter_map(|row| row.thumb.clone())
                .filter(|path| crate::config::ServerUrl::is_path(path))
                .collect()
        });
        let room = self.room;
        for path in paths {
            self.fetch_picture(room, path, cx);
        }
    }

    /// Writes the picks to `needs.json`, off the UI thread; a failure is logged (the pick still
    /// holds for this launch).
    fn keep_needs_view(&mut self, cx: &mut Context<'_, Self>) {
        cx.notify();
        let Some(files) = &self.files else {
            return;
        };
        let (path, kept) = (path(files), self.needs_view.kept.clone());
        cx.background_spawn(async move {
            if let Err(error) = write(&path, &kept) {
                log::warn!(
                    "can't keep how Needs you looks in {}: {error}",
                    path.display()
                );
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use emberhut_api::ids::TaskId;

    use super::*;
    use crate::fixtures::{issue, state, waiting};
    use crate::wire::ASSISTANT_OWNER;

    const TODAY: jiff::civil::Date = jiff::civil::date(2026, 11, 25);

    #[test]
    fn each_part_of_the_filter_narrows_the_asks() {
        let rows = [
            waiting(1, json!({"pid": 2, "area": 9, "decideBy": "2026-11-25"})),
            waiting(2, json!({"decideBy": "2026-12-10"})),
        ];
        let lists = state(json!({
            "issues": [issue(1, "waiting", json!({"owner": "owner"})),
                       issue(2, "waiting", json!({"owner": ASSISTANT_OWNER}))],
            "working": rows,
        }));
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let kept = |filter: &NeedsFilter| -> Vec<i64> {
            lists
                .working
                .iter()
                .filter(|row| filter.keeps(row, &needs, &lists))
                .filter_map(|row| row.task.map(TaskId::get))
                .collect()
        };
        assert_eq!(kept(&NeedsFilter::default()), [1, 2]);
        let project = NeedsFilter {
            project: Some(ProjectPick::NoProject),
            ..NeedsFilter::default()
        };
        assert_eq!(kept(&project), [2]);
        let area = NeedsFilter {
            area: Some(AreaId::new(9)),
            ..NeedsFilter::default()
        };
        assert_eq!(kept(&area), [1]);
        let owner = NeedsFilter {
            owner: Some(OwnerPick::Assistants),
            ..NeedsFilter::default()
        };
        assert_eq!(kept(&owner), [2]);
        let when = NeedsFilter {
            when: Some(When::Today),
            ..NeedsFilter::default()
        };
        assert_eq!(kept(&when), [1]);
        assert_eq!(
            when.shown(false, Layout::Board),
            NeedsFilter::default(),
            "the board doesn't narrow by when"
        );
        assert_eq!(
            area.shown(true, Layout::List),
            NeedsFilter::default(),
            "projects hidden: no area either"
        );
    }

    #[test]
    fn only_the_projects_and_areas_with_asks_are_offered() {
        let rows = [
            waiting(1, json!({"pid": 2, "area": 9})),
            waiting(2, json!({})),
            waiting(3, json!({"pid": 2})),
        ];
        let refs: Vec<&WorkRow> = rows.iter().collect();
        assert_eq!(projects_of(&refs), [Some(ProjectId::new(2)), None]);
        let area = |id: i64| json!({"id": id, "pid": 2, "name": "Area", "status": "active", "state": "working"});
        let lists = state(json!({"areas": [area(8), area(9)]}));
        assert_eq!(areas_of(&refs, &lists), [AreaId::new(9)]);
    }

    #[test]
    fn later_starts_folded_and_a_click_turns_a_column_the_other_way() {
        let mut view = NeedsView::default();
        let later = ColumnKey::When(When::Later);
        let project = ColumnKey::Project(Some(ProjectId::new(2)));
        assert!(view.is_folded(&later) && !view.is_folded(&project));
        view.flip(later.clone());
        view.flip(project.clone());
        assert!(!view.is_folded(&later) && view.is_folded(&project));
        view.flip(project.clone());
        assert!(!view.is_folded(&project));
    }

    #[test]
    fn the_picks_are_kept_on_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("app"));
        let view = NeedsView::load(Some(&files));
        assert_eq!(view.layout(), Layout::Board, "the board by default");
        assert_eq!(view.grouping(), Grouping::Project);
        let kept = Kept {
            layout: Layout::List,
            grouping: Grouping::When,
            filter: NeedsFilter {
                owner: Some(OwnerPick::Yours),
                ..NeedsFilter::default()
            },
            project_layout: Layout::List,
        };
        write(&path(&files), &kept).unwrap();
        let view = NeedsView::load(Some(&files));
        assert_eq!(
            (view.layout(), view.grouping(), view.filter().clone()),
            (Layout::List, Grouping::When, kept.filter)
        );
        std::fs::write(path(&files), "not json").unwrap();
        assert_eq!(NeedsView::load(Some(&files)).layout(), Layout::Board);
    }
}
