//! A project's page: how far it is, what of it needs the user, and its work by area (its
//! active areas, each a lane of its tasks with their steps nested, then the tasks in no area),
//! narrowed by area, state and owner; and the changes the user makes to it (priority,
//! deadline), sent at once.

use std::sync::Arc;

use emberhut_api::areas::{Area, AreaStatus, TreeNode, WorkStage};
use emberhut_api::ids::{AreaId, ProjectId, TaskId};
use emberhut_api::patch::Patch;
use emberhut_api::settings::{ProjectChange, SettingsAction};
use emberhut_api::state::State;
use emberhut_api::task_live::LiveState;
use emberhut_api::tasks::{Issue, Project, TaskStatus};
use emberhut_api::time::Day;
use emberhut_api::work::WorkRow;
use gpui_kit::{AppContext as _, Context};

use super::needs::Needs;
use super::tasks::filter::OwnerPick;
use super::{Store, lookup, update_or_log};

/// The suffix of an area's own task (the work that stands for the area): its steps sit in
/// the lane instead of it (the reference server's convention for such tasks).
const AREA_TASK: &str = "(area)";

/// The priorities a project can have, in the menu's order, with their names.
pub(crate) const PRIORITIES: [(u8, &str); 5] = [
    (1, "Urgent"),
    (2, "High"),
    (3, "Medium"),
    (4, "Low"),
    (0, "No priority"),
];

/// A priority's name.
#[must_use]
pub(crate) fn priority_name(priority: i32) -> &'static str {
    PRIORITIES
        .iter()
        .find(|(value, _)| i32::from(*value) == priority)
        .map_or("No priority", |(_, name)| name)
}

/// Where a task stands on a project's board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Waiting on the user.
    NeedsYou,
    /// Not started.
    NotStarted,
    /// Being worked on.
    Working,
    /// Nothing runs until its next look.
    Scheduled,
    /// Done.
    Done,
}

impl Stage {
    /// The board's columns, in order.
    pub(crate) const ALL: [Self; 5] = [
        Self::NeedsYou,
        Self::NotStarted,
        Self::Working,
        Self::Scheduled,
        Self::Done,
    ];

    /// Its column's name.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::NotStarted => "Not started",
            Self::Working => "Working",
            Self::Scheduled => "Scheduled",
            Self::Done => "Done",
        }
    }

    /// A tree node's stage; `scheduled` when the state says nothing runs until its next look.
    fn of_stage(stage: &WorkStage, scheduled: bool) -> Self {
        match stage {
            WorkStage::Done => Self::Done,
            WorkStage::NeedsYou => Self::NeedsYou,
            WorkStage::NotStarted => Self::NotStarted,
            _ if scheduled => Self::Scheduled,
            _ => Self::Working,
        }
    }

    /// A task's stage.
    fn of_task(task: &Issue) -> Self {
        if task
            .live
            .as_ref()
            .is_some_and(|live| live.state == LiveState::Scheduled)
        {
            return Self::Scheduled;
        }
        match task.status {
            TaskStatus::Waiting => Self::NeedsYou,
            TaskStatus::Todo => Self::NotStarted,
            TaskStatus::Done | TaskStatus::Canceled => Self::Done,
            _ => Self::Working,
        }
    }
}

/// A task on a project's page, its steps under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    /// The task.
    pub(crate) id: TaskId,
    /// Its title.
    pub(crate) title: String,
    /// What the user should do.
    pub(crate) action: Option<String>,
    /// Its day (to decide by, else due).
    pub(crate) day: Option<Day>,
    /// Where it stands.
    pub(crate) stage: Stage,
    /// Whose it is: the assistant's.
    pub(crate) assistants: bool,
    /// Its steps.
    pub(crate) kids: Vec<Item>,
}

impl Item {
    /// Its leaves done of all (`3/5` on its card).
    #[must_use]
    pub(crate) fn leaves(&self) -> (usize, usize) {
        if self.kids.is_empty() {
            return (usize::from(self.stage == Stage::Done), 1);
        }
        self.kids.iter().fold((0, 0), |(done, total), kid| {
            let (d, t) = kid.leaves();
            (done + d, total + t)
        })
    }
}

/// A lane of a project's page: an area's work, or the tasks in no area.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lane<'a> {
    /// Its title: the area's name, `Not in an area`, or `Tasks` when it has no areas.
    pub(crate) title: String,
    /// Its area.
    pub(crate) area: Option<&'a Area>,
    /// Its work.
    pub(crate) items: Vec<Item>,
}

/// The page's filter: by area, state and owner. State and owner keep a task when it or a step
/// under it matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProjectFilter {
    /// One area.
    pub(crate) area: Option<AreaId>,
    /// One stage (Working takes Scheduled too).
    pub(crate) stage: Option<Stage>,
    /// One owner.
    pub(crate) owner: Option<OwnerPick>,
}

impl ProjectFilter {
    /// Whether it narrows nothing.
    #[must_use]
    pub(crate) fn is_empty(self) -> bool {
        self == Self::default()
    }

    fn keeps(self, item: &Item) -> bool {
        let stage = self.stage.is_none_or(|stage| {
            item.stage == stage || (stage == Stage::Working && item.stage == Stage::Scheduled)
        });
        let owner = self
            .owner
            .is_none_or(|owner| (owner == OwnerPick::Assistants) == item.assistants);
        stage && owner
    }

    /// `item` as it shows: kept whole when it matches, with only the matching steps when one
    /// under it does, else gone.
    fn cut(self, item: &Item) -> Option<Item> {
        if self.stage.is_none() && self.owner.is_none() {
            return Some(item.clone());
        }
        if self.keeps(item) {
            return Some(item.clone());
        }
        let kids: Vec<Item> = item.kids.iter().filter_map(|kid| self.cut(kid)).collect();
        (!kids.is_empty()).then(|| Item {
            kids,
            ..item.clone()
        })
    }
}

/// A project's page over one state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectPage<'a> {
    /// The state.
    pub(crate) state: &'a State,
    /// The project.
    pub(crate) project: &'a Project,
}

impl<'a> ProjectPage<'a> {
    /// The page of `project` in `state`, if the state has it.
    #[must_use]
    pub(crate) fn of(state: &'a State, project: ProjectId) -> Option<Self> {
        let project = state.projects.iter().find(|p| p.id == project)?;
        Some(Self { state, project })
    }

    /// Its areas, every status.
    pub(crate) fn areas(&self) -> impl Iterator<Item = &'a Area> + '_ {
        self.state
            .areas
            .iter()
            .filter(|area| area.pid == self.project.id)
    }

    /// How far it is: its active areas' done and this week's, of those and what is left.
    #[must_use]
    pub(crate) fn progress(&self) -> (u32, u32) {
        let active: Vec<&Area> = self
            .areas()
            .filter(|area| area.status == AreaStatus::Active)
            .collect();
        let done = active.iter().map(|area| area.counts.done).sum::<u32>() + self.project.done_week;
        let left: u32 = active.iter().map(|area| area.open()).sum();
        (done, done + self.project.open + left)
    }

    /// What of it needs the user, soonest to decide first.
    #[must_use]
    pub(crate) fn needs(&self, needs: &Needs<'a>) -> Vec<&'a WorkRow> {
        let mut rows: Vec<&WorkRow> = needs
            .rows()
            .into_iter()
            .filter(|row| row.pid == Some(self.project.id))
            .collect();
        rows.sort_by_key(|row| {
            (
                lookup::decide_by(row).map_or_else(|| "9".to_owned(), str::to_owned),
                lookup::asked(row).cloned(),
            )
        });
        rows
    }

    /// Its work by lane: each active area (or the one picked), then its tasks in no area, open
    /// ones then the done ones (`done`: all of them, as fetched; else this week's the state
    /// has), each cut to the filter; a lane left empty is out.
    #[must_use]
    pub(crate) fn lanes(
        &self,
        filter: ProjectFilter,
        done: Option<&'a [Issue]>,
        gone: &dyn Fn(TaskId) -> bool,
    ) -> Vec<Lane<'a>> {
        let scheduled: Vec<TaskId> = self
            .state
            .issues
            .iter()
            .filter(|task| {
                task.live
                    .as_ref()
                    .is_some_and(|live| live.state == LiveState::Scheduled)
            })
            .map(|task| task.id)
            .collect();
        let areas: Vec<&Area> = self
            .areas()
            .filter(|area| {
                filter
                    .area
                    .map_or(area.status == AreaStatus::Active, |id| area.id == id)
            })
            .collect();
        let mut lanes: Vec<Lane<'a>> = areas
            .iter()
            .map(|area| Lane {
                title: area.name.clone(),
                area: Some(area),
                items: unwrap(
                    area.tree
                        .iter()
                        .map(|node| node_item(node, &scheduled))
                        .collect(),
                ),
            })
            .collect();
        if filter.area.is_none() {
            let pid = Some(self.project.id);
            let in_project = |task: &&Issue| {
                task.pid == pid
                    && task.area.is_none()
                    && task.parent.is_none()
                    && !task.is_finding()
            };
            let open = self
                .state
                .issues
                .iter()
                .filter(in_project)
                .filter(|task| task.is_open() && !gone(task.id));
            let finished = done
                .unwrap_or(&self.state.issues)
                .iter()
                .filter(in_project)
                .filter(|task| task.status == TaskStatus::Done);
            let items: Vec<Item> = open
                .chain(finished)
                .map(|task| self.task_item(task))
                .collect();
            if !items.is_empty() {
                lanes.push(Lane {
                    title: if areas.is_empty() {
                        "Tasks".to_owned()
                    } else {
                        "Not in an area".to_owned()
                    },
                    area: None,
                    items,
                });
            }
        }
        lanes
            .into_iter()
            .filter_map(|lane| {
                let items: Vec<Item> = lane
                    .items
                    .iter()
                    .filter_map(|item| filter.cut(item))
                    .collect();
                (!items.is_empty()).then_some(Lane { items, ..lane })
            })
            .collect()
    }

    /// Its areas that aren't active: paused and done, folded at the foot.
    #[must_use]
    pub(crate) fn quiet_areas(&self) -> Vec<&'a Area> {
        self.areas()
            .filter(|area| area.status != AreaStatus::Active)
            .collect()
    }

    fn task_item(&self, task: &Issue) -> Item {
        Item {
            id: task.id,
            title: task.title.clone(),
            action: task.action.clone(),
            day: task.decide_by.clone().or_else(|| task.due.clone()),
            stage: Stage::of_task(task),
            assistants: lookup::is_assistants(task),
            kids: self
                .state
                .issues
                .iter()
                .filter(|kid| kid.parent == Some(task.id) && kid.status != TaskStatus::Canceled)
                .map(|kid| self.task_item(kid))
                .collect(),
        }
    }
}

/// A tree node as an item.
fn node_item(node: &TreeNode, scheduled: &[TaskId]) -> Item {
    Item {
        id: node.id,
        title: node.title.clone(),
        action: node.action.clone(),
        day: node.due.clone(),
        stage: Stage::of_stage(&node.state, scheduled.contains(&node.id)),
        assistants: node.owner == crate::wire::ASSISTANT_OWNER,
        kids: node
            .kids
            .iter()
            .map(|kid| node_item(kid, scheduled))
            .collect(),
    }
}

/// An area's own task isn't a row of its own: its steps sit in the lane.
fn unwrap(items: Vec<Item>) -> Vec<Item> {
    items
        .into_iter()
        .flat_map(|item| {
            if item.title.ends_with(AREA_TASK) && !item.kids.is_empty() {
                item.kids
            } else {
                vec![item]
            }
        })
        .collect()
}

impl Store {
    /// Changes `project` (its priority, deadline, whether that is hard); sent at once, and a
    /// refusal says why.
    pub(crate) fn change_project(&mut self, change: ProjectChange, cx: &mut Context<'_, Self>) {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(
                    async move { backend.settings(SettingsAction::ProjectSet(change)) },
                )
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = done {
                    store.action_failed("Project not changed", &error, cx);
                }
            });
        })
        .detach();
    }
}

/// A change of `project` that changes nothing yet.
#[must_use]
pub(crate) fn change_of(project: ProjectId) -> ProjectChange {
    ProjectChange {
        id: project,
        priority: None,
        deadline: Patch::Keep,
        hard: None,
        status: None,
        paused_until: None,
    }
}

/// How far out a deadline starts when it is turned on: a week from today.
pub(crate) const DEADLINE_STARTS: i8 = 7;

/// The change that sets `project`'s deadline to `day` (none: no deadline) and whether it is
/// hard: someone else holds the user to it. A project with no deadline has no hard one.
#[must_use]
pub(crate) fn deadline_change(
    project: ProjectId,
    day: Option<jiff::civil::Date>,
    hard: bool,
) -> ProjectChange {
    ProjectChange {
        deadline: day.map_or(Patch::Clear, |day| {
            Patch::Set(Day::new(day.strftime("%Y-%m-%d").to_string()))
        }),
        hard: Some(hard && day.is_some()),
        ..change_of(project)
    }
}

/// The day a deadline turned on starts at, from `today`.
#[must_use]
pub(crate) fn deadline_start(today: jiff::civil::Date) -> jiff::civil::Date {
    today
        .checked_add(jiff::Span::new().days(DEADLINE_STARTS))
        .unwrap_or(today)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, state, waiting};
    use crate::wire::ASSISTANT_OWNER;

    #[test]
    fn a_deadline_is_set_with_its_hardness_and_cleared_with_it() {
        let day = jiff::civil::date(2026, 12, 3);
        let set = deadline_change(ProjectId::new(2), Some(day), true);
        assert_eq!(set.deadline, Patch::Set(Day::new("2026-12-03")));
        assert_eq!(set.hard, Some(true));
        let cleared = deadline_change(ProjectId::new(2), None, true);
        assert_eq!(cleared.deadline, Patch::Clear);
        assert_eq!(cleared.hard, Some(false), "no deadline, nothing hard");
        assert_eq!(
            deadline_start(jiff::civil::date(2026, 11, 26)),
            jiff::civil::date(2026, 12, 3)
        );
    }

    fn project() -> serde_json::Value {
        json!({"id": 2, "name": "Example", "goal": null, "priority": 1, "kind": "deliverable",
            "deadline": null, "hard": false, "status": "active", "pausedUntil": null, "open": 2,
            "doneWeek": 1, "health": null, "next": null, "hoursLeft": 0.0})
    }

    fn node(id: i64, title: &str, state: &str, kids: &serde_json::Value) -> serde_json::Value {
        json!({"id": id, "title": title, "owner": "owner", "status": "todo", "state": state, "kids": kids})
    }

    fn lists() -> State {
        state(json!({
            "projects": [project()],
            "areas": [
                {"id": 9, "pid": 2, "name": "Area A", "status": "active", "state": "working",
                 "counts": {"done": 1, "working": 1, "needsYou": 0, "notStarted": 1, "total": 3},
                 "tree": [node(10, "Area A (area)", "working", &json!([
                    node(11, "Step one", "done", &json!([])),
                    node(12, "Step two", "working", &json!([]))])),
                          node(13, "Loose", "not_started", &json!([]))]},
                {"id": 8, "pid": 2, "name": "Old", "status": "done", "state": "done"}
            ],
            "issues": [
                issue(20, "todo", json!({"pid": 2, "owner": ASSISTANT_OWNER})),
                issue(21, "done", json!({"pid": 2})),
                issue(22, "todo", json!({"pid": 2, "area": 9})),
                issue(23, "todo", json!({"pid": 3})),
            ],
            "working": [waiting(20, json!({"pid": 2, "decideBy": "2026-12-01"})),
                        waiting(24, json!({"pid": 2, "decideBy": "2026-11-27"}))],
        }))
    }

    #[test]
    fn its_progress_counts_its_areas_and_its_week() {
        let lists = lists();
        let page = ProjectPage::of(&lists, ProjectId::new(2)).unwrap();
        assert_eq!(
            page.progress(),
            (2, 6),
            "1 + 1 done, 2 open, 2 left in the area"
        );
        assert_eq!(page.quiet_areas().len(), 1);
    }

    #[test]
    fn its_lanes_are_its_areas_then_the_tasks_in_none() {
        let lists = lists();
        let page = ProjectPage::of(&lists, ProjectId::new(2)).unwrap();
        let lanes = page.lanes(ProjectFilter::default(), None, &|_| false);
        let shape: Vec<(&str, Vec<i64>)> = lanes
            .iter()
            .map(|lane| {
                (
                    lane.title.as_str(),
                    lane.items.iter().map(|i| i.id.get()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("Area A", vec![11, 12, 13]),
                ("Not in an area", vec![20, 21])
            ],
            "the area's own task gives way to its steps"
        );
        let mine = ProjectFilter {
            owner: Some(OwnerPick::Assistants),
            ..ProjectFilter::default()
        };
        let lanes = page.lanes(mine, None, &|_| false);
        assert_eq!(
            lanes.len(),
            1,
            "only the lane with the assistant's task is left"
        );
        let working = ProjectFilter {
            stage: Some(Stage::Working),
            ..ProjectFilter::default()
        };
        let lanes = page.lanes(working, None, &|_| false);
        assert_eq!(lanes[0].items.len(), 1);
        let gone = page.lanes(ProjectFilter::default(), None, &|task| {
            task == TaskId::new(20)
        });
        assert_eq!(gone[1].items.len(), 1, "a task acted on here is gone");
    }

    #[test]
    fn its_needs_are_soonest_first_and_a_parents_steps_count_as_leaves() {
        let lists = lists();
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, jiff::civil::date(2026, 11, 25));
        let page = ProjectPage::of(&lists, ProjectId::new(2)).unwrap();
        let rows: Vec<i64> = page
            .needs(&needs)
            .iter()
            .filter_map(|row| row.task.map(TaskId::get))
            .collect();
        assert_eq!(rows, [24, 20]);
        let parent = Item {
            id: TaskId::new(1),
            title: String::new(),
            action: None,
            day: None,
            stage: Stage::Working,
            assistants: false,
            kids: vec![
                Item {
                    stage: Stage::Done,
                    kids: Vec::new(),
                    ..parent_stub()
                },
                parent_stub(),
            ],
        };
        assert_eq!(parent.leaves(), (1, 2));
        assert_eq!(priority_name(1), "Urgent");
        assert_eq!(priority_name(0), "No priority");
    }

    fn parent_stub() -> Item {
        Item {
            id: TaskId::new(2),
            title: String::new(),
            action: None,
            day: None,
            stage: Stage::NotStarted,
            assistants: false,
            kids: Vec::new(),
        }
    }
}
