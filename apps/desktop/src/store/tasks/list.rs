//! The Tasks list as one flat list: what needs the user, then (on a server that keeps plans)
//! what runs now and what is blocked, then today's (ranked by the server), then the rest in project order and then
//! the server's. Tasks of paused or archived projects
//! wait off the list; a parent shows once, its subtasks under it on demand. The filter narrows
//! it, and says how many it hides.

use std::collections::HashSet;

use emberhut_api::ids::{ProjectId, TaskId};
use emberhut_api::state::State;
use emberhut_api::tasks::{Issue, ProjectStatus, TaskStatus};
use jiff::civil::Date;

use super::filter::{StatusPick, TaskFilter};
use super::words::{self, Doing};
use crate::store::lookup;

/// The list's sections.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TaskList<'a> {
    /// Waiting on the user.
    pub(crate) needs_you: Vec<&'a Issue>,
    /// Running or queued now, on a server that keeps plans (else they stay in their section).
    pub(crate) running: Vec<&'a Issue>,
    /// Not running and blocked, by its plan, on a server that keeps plans.
    pub(crate) blocked: Vec<&'a Issue>,
    /// Today's, in the server's order.
    pub(crate) today: Vec<&'a Issue>,
    /// The rest, by project order.
    pub(crate) rest: Vec<&'a Issue>,
    /// How many open tasks the filter hides.
    pub(crate) hidden: usize,
    /// Whether a filter is on.
    pub(crate) filtered: bool,
}

impl<'a> TaskList<'a> {
    /// The list over `state`, without the tasks acted on here (`gone`), narrowed by `filter`
    /// as it stands on `today`.
    #[must_use]
    pub(crate) fn new(
        state: &'a State,
        gone: &HashSet<TaskId>,
        filter: &TaskFilter,
        today: Date,
    ) -> Self {
        let tops: Vec<&Issue> = state
            .issues
            .iter()
            .filter(|task| task.status != TaskStatus::Canceled && task.parent.is_none())
            .filter(|task| !gone.contains(&task.id))
            .collect();
        let unfiltered: Vec<&Issue> = tops
            .iter()
            .copied()
            .filter(|task| task.status != TaskStatus::Done)
            .collect();
        let open: Vec<&Issue> = if filter.is_empty() {
            unfiltered.clone()
        } else {
            // The Done pick pulls in this week's done tasks.
            let from = if filter.status == Some(StatusPick::Done) {
                &tops
            } else {
                &unfiltered
            };
            from.iter()
                .copied()
                .filter(|task| filter.keeps(task, today))
                .collect()
        };
        let order: Vec<ProjectId> = state
            .projects
            .iter()
            .filter(|project| project.status == ProjectStatus::Active)
            .map(|project| project.id)
            .collect();
        let shows = |task: &Issue| task.pid.is_none_or(|pid| order.contains(&pid));
        let waits = |task: &Issue| task.status == TaskStatus::Waiting;
        let needs_you = open
            .iter()
            .copied()
            .filter(|task| waits(task) && shows(task))
            .collect();
        let plans = lookup::keeps_plans(state);
        let runs = |task: &Issue| {
            plans && !waits(task) && matches!(words::doing(task), Doing::Running | Doing::Queued)
        };
        let running = open
            .iter()
            .copied()
            .filter(|task| runs(task) && shows(task))
            .collect();
        let stuck = |task: &Issue| {
            plans
                && !waits(task)
                && !runs(task)
                && task
                    .plan
                    .as_ref()
                    .is_some_and(|plan| plan.blocker.is_some())
        };
        let blocked = open
            .iter()
            .copied()
            .filter(|task| stuck(task) && shows(task))
            .collect();
        let aside = |task: &Issue| waits(task) || runs(task) || stuck(task);
        let mut ranked = HashSet::new();
        let today_tasks: Vec<&Issue> = state
            .today
            .iter()
            .filter(|&&id| ranked.insert(id))
            .filter_map(|&id| {
                open.iter()
                    .copied()
                    .find(|task| task.id == id && !aside(task))
            })
            .collect();
        let rank = |task: &Issue| {
            task.pid
                .and_then(|pid| order.iter().position(|&id| id == pid))
                .unwrap_or(order.len())
        };
        let mut rest: Vec<&Issue> = open
            .iter()
            .copied()
            .filter(|task| !aside(task) && !ranked.contains(&task.id) && shows(task))
            .collect();
        // Stable: within a project, the server's order stays.
        rest.sort_by_key(|task| rank(task));
        let hidden = if filter.is_empty() {
            0
        } else {
            unfiltered
                .iter()
                .filter(|task| !filter.keeps(task, today))
                .count()
        };
        Self {
            needs_you,
            running,
            blocked,
            today: today_tasks,
            rest,
            hidden,
            filtered: !filter.is_empty(),
        }
    }

    /// The rest's heading: `Open` when it is all there is, else `The rest`.
    #[must_use]
    pub(crate) fn rest_heading(&self) -> &'static str {
        if self.needs_you.is_empty()
            && self.running.is_empty()
            && self.blocked.is_empty()
            && self.today.is_empty()
        {
            "Open"
        } else {
            "The rest"
        }
    }

    /// Whether no section has a row.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.needs_you.is_empty()
            && self.running.is_empty()
            && self.blocked.is_empty()
            && self.today.is_empty()
            && self.rest.is_empty()
    }

    /// What an empty list says.
    #[must_use]
    pub(crate) fn empty_text(&self) -> &'static str {
        if self.filtered {
            "Nothing here for this filter."
        } else {
            "Nothing open."
        }
    }

    /// How many open tasks there are, every section counted.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.needs_you.len()
            + self.running.len()
            + self.blocked.len()
            + self.today.len()
            + self.rest.len()
    }
}

/// `parent`'s subtasks in the state, in their order, without those acted on here.
#[must_use]
pub(crate) fn subtasks<'a>(
    state: &'a State,
    parent: TaskId,
    gone: &HashSet<TaskId>,
) -> Vec<&'a Issue> {
    let mut subtasks: Vec<&Issue> = state
        .issues
        .iter()
        .filter(|task| task.parent == Some(parent) && !gone.contains(&task.id))
        .collect();
    subtasks.sort_by_key(|task| task.id);
    subtasks
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixtures::{issue, state};
    use crate::store::tasks::filter::OwnerPick;

    const TODAY: Date = jiff::civil::date(2026, 11, 25);

    fn project(id: i64, status: &str) -> Value {
        json!({"id": id, "name": format!("P{id}"), "goal": null, "priority": 0, "kind": "ongoing",
            "deadline": null, "hard": false, "status": status, "pausedUntil": null, "open": 0,
            "doneWeek": 0, "health": null, "next": null, "hoursLeft": 0.0})
    }

    fn ids(tasks: &[&Issue]) -> Vec<i64> {
        tasks.iter().map(|task| task.id.get()).collect()
    }

    fn tasks() -> State {
        state(json!({
            "projects": [project(7, "active"), project(5, "active"), project(9, "paused")],
            "issues": [
                issue(1, "waiting", json!({})),
                issue(2, "todo", json!({"pid": 5})),
                issue(3, "working", json!({"pid": 7})),
                issue(4, "todo", json!({})),
                issue(5, "todo", json!({"pid": 9})),
                issue(6, "done", json!({})),
                issue(7, "canceled", json!({})),
                issue(8, "todo", json!({"pid": 7, "parent": 3})),
                issue(9, "todo", json!({"pid": 7})),
            ],
            "today": [9, 1, 9],
        }))
    }

    #[test]
    fn needs_you_then_today_then_the_rest_in_project_order() {
        let state = tasks();
        let list = TaskList::new(&state, &HashSet::new(), &TaskFilter::default(), TODAY);
        assert_eq!(ids(&list.needs_you), [1]);
        assert_eq!(
            ids(&list.today),
            [9],
            "a waiting task shows once, under Needs you"
        );
        assert_eq!(
            ids(&list.rest),
            [3, 2, 4],
            "project order, then tasks of no project; paused projects' tasks wait"
        );
        assert_eq!(list.rest_heading(), "The rest");
        assert_eq!(list.len(), 5);
        assert_eq!(list.hidden, 0);
    }

    #[test]
    fn tasks_acted_on_leave_the_list_at_once() {
        let state = tasks();
        let gone = HashSet::from([TaskId::new(1), TaskId::new(9)]);
        let list = TaskList::new(&state, &gone, &TaskFilter::default(), TODAY);
        assert!(list.needs_you.is_empty() && list.today.is_empty());
        assert_eq!(list.rest_heading(), "Open");
    }

    #[test]
    fn a_filter_narrows_the_list_and_counts_what_it_hides() {
        let state = tasks();
        let filter = TaskFilter {
            owner: Some(OwnerPick::Assistants),
            ..TaskFilter::default()
        };
        let list = TaskList::new(&state, &HashSet::new(), &filter, TODAY);
        assert!(list.is_empty());
        assert_eq!(list.empty_text(), "Nothing here for this filter.");
        assert_eq!(list.hidden, 6, "every open top-level task is the user's");

        let done = TaskFilter {
            status: Some(StatusPick::Done),
            ..TaskFilter::default()
        };
        let list = TaskList::new(&state, &HashSet::new(), &done, TODAY);
        assert_eq!(ids(&list.rest), [6], "Done pulls in the done ones");
    }

    #[test]
    fn an_empty_list_says_nothing_is_open() {
        let empty = state(json!({}));
        let list = TaskList::new(&empty, &HashSet::new(), &TaskFilter::default(), TODAY);
        assert!(list.is_empty());
        assert_eq!(list.empty_text(), "Nothing open.");
    }

    #[test]
    fn with_plans_what_runs_has_its_own_section() {
        let running = json!({"live": {"state": "running"}});
        let queued = json!({"live": {"state": "queued"}, "pid": 7});
        let mut tasks = vec![
            issue(1, "waiting", json!({"live": {"state": "waiting"}})),
            issue(2, "working", running.clone()),
            issue(3, "working", queued),
            issue(4, "todo", json!({})),
            issue(
                5,
                "working",
                json!({"live": {"state": "running"}, "pid": 9}),
            ),
        ];
        let plain = state(
            json!({"projects": [project(7, "active"), project(9, "paused")],
            "issues": tasks.clone(), "today": [2]}),
        );
        let list = TaskList::new(&plain, &HashSet::new(), &TaskFilter::default(), TODAY);
        assert!(
            list.running.is_empty() && list.blocked.is_empty(),
            "no plans: the list is as it was"
        );
        assert_eq!(ids(&list.today), [2]);
        tasks.push(issue(6, "working", running));
        tasks.push(issue(
            7,
            "todo",
            json!({"plan": {"done": 1, "total": 3, "blocker": "No token"}}),
        ));
        let plans = state(
            json!({"projects": [project(7, "active"), project(9, "paused")],
            "issues": tasks, "today": [2], "ui": {"taskPlans": true}}),
        );
        let list = TaskList::new(&plans, &HashSet::new(), &TaskFilter::default(), TODAY);
        assert_eq!(ids(&list.needs_you), [1]);
        assert_eq!(
            ids(&list.running),
            [2, 3, 6],
            "running or queued, a paused project's left out"
        );
        assert_eq!(ids(&list.blocked), [7]);
        assert_eq!(ids(&list.today), [0_i64; 0]);
        assert_eq!(ids(&list.rest), [4]);
        assert_eq!(list.rest_heading(), "The rest");
        assert_eq!(list.len(), 6);
    }

    #[test]
    fn subtasks_come_in_their_order() {
        let state = state(json!({"issues": [
            issue(12, "todo", json!({"parent": 3})),
            issue(11, "done", json!({"parent": 3})),
            issue(13, "todo", json!({"parent": 4})),
        ]}));
        assert_eq!(
            ids(&subtasks(&state, TaskId::new(3), &HashSet::new())),
            [11, 12]
        );
        let gone = HashSet::from([TaskId::new(11)]);
        assert_eq!(ids(&subtasks(&state, TaskId::new(3), &gone)), [12]);
    }
}
