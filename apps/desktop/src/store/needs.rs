//! What waits on the user: the asks on Needs you (by project or by when they have to be
//! decided, then Later), their count on the sidebar, and the open tasks. A task acted on here
//! leaves every list at once ("gone") until the server agrees or the user takes it back.
//!
//! The most urgent ask comes first: by when it has to be decided (overdue first), then the
//! cheapest to answer (a quick yes, a read and a pick, something only the user can do), then
//! the newest asked.

use std::cmp::Reverse;
use std::collections::HashSet;

use emberhut_api::ids::{ProjectId, TaskId};
use emberhut_api::state::State;
use emberhut_api::tasks::{AskKind, Issue, TaskStatus};
use emberhut_api::time::Timestamp;
use emberhut_api::work::WorkRow;
use jiff::civil::Date;

use super::lookup;
use super::tasks::filter::When;
use crate::wire::TASK_KEY_PREFIX;

/// The lists over one state, on one day.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Needs<'a> {
    state: &'a State,
    gone: &'a HashSet<TaskId>,
    today: Date,
}

/// One project's asks on Needs you.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NeedsGroup<'a> {
    /// The project; `None` for asks with no project (or one the state doesn't have).
    pub(crate) project: Option<ProjectId>,
    /// Its name, `Other` for no project.
    pub(crate) name: String,
    /// Its asks, soonest to decide first.
    pub(crate) rows: Vec<&'a WorkRow>,
}

impl<'a> Needs<'a> {
    /// The lists over `state` as they stand on `today`, with the tasks in `gone` left out.
    #[must_use]
    pub(crate) fn new(state: &'a State, gone: &'a HashSet<TaskId>, today: Date) -> Self {
        Self { state, gone, today }
    }

    /// The day the lists stand on: the decide-by days are seen from it.
    #[must_use]
    pub(crate) fn today(&self) -> Date {
        self.today
    }

    /// What waits on the user: the work rows waiting on them, copies out.
    #[must_use]
    pub(crate) fn rows(&self) -> Vec<&'a WorkRow> {
        let waiting: Vec<&WorkRow> = self
            .state
            .working
            .iter()
            .filter(|row| row.needs_you() && self.waits_on_you(row.task))
            .collect();
        waiting
            .iter()
            .copied()
            .filter(|row| !self.is_copy(row, &waiting))
            .collect()
    }

    /// The sidebar's Needs you count: the rows, Later left out.
    #[must_use]
    pub(crate) fn count(&self) -> usize {
        self.rows().iter().filter(|row| !self.is_later(row)).count()
    }

    /// Needs you by project, over `rows` (the asks a filter keeps): each project's asks most
    /// urgent first, the projects in the order of their most urgent ask, asks with no known
    /// project last as Other. Later is left out: it has its own fold ([`Needs::later_of`]).
    #[must_use]
    pub(crate) fn by_project_of(&self, rows: Vec<&'a WorkRow>) -> Vec<NeedsGroup<'a>> {
        let mut open: Vec<&WorkRow> = rows.into_iter().filter(|row| !self.is_later(row)).collect();
        open.sort_by_cached_key(|row| self.urgency(row));
        let mut groups: Vec<NeedsGroup<'a>> = Vec::new();
        for row in open {
            let name = lookup::project_name(self.state, row.pid);
            let project = name.and(row.pid);
            match groups.iter_mut().find(|group| group.project == project) {
                Some(group) => group.rows.push(row),
                None => groups.push(NeedsGroup {
                    project,
                    name: name.unwrap_or("Other").to_owned(),
                    rows: vec![row],
                }),
            }
        }
        // Stable: the projects keep their order, Other goes last.
        groups.sort_by_key(|group| group.project.is_none());
        groups
    }

    /// Later, over `rows`: the asks in paused projects or to be decided more than a month out,
    /// most urgent first. They wait quietly in a fold after the projects and count nowhere.
    #[must_use]
    pub(crate) fn later_of(&self, rows: Vec<&'a WorkRow>) -> Vec<&'a WorkRow> {
        let mut later: Vec<&WorkRow> = rows.into_iter().filter(|row| self.is_later(row)).collect();
        later.sort_by_cached_key(|row| self.urgency(row));
        later
    }

    /// Needs you by when the asks have to be decided: Today (the overdue ones too), This week,
    /// This month, each the cheapest to answer first, then the soonest, then the newest asked;
    /// the empty ones left out, and Later too (its own fold).
    #[must_use]
    pub(crate) fn by_when_of(&self, rows: &[&'a WorkRow]) -> Vec<(When, Vec<&'a WorkRow>)> {
        let mut sorted = rows.to_vec();
        sorted.sort_by_cached_key(|row| {
            (
                kind_rank(row),
                lookup::decide_by(row).map(str::to_owned),
                Reverse(lookup::asked(row).cloned()),
            )
        });
        [When::Today, When::ThisWeek, When::ThisMonth]
            .into_iter()
            .map(|when| {
                let group: Vec<&WorkRow> = sorted
                    .iter()
                    .copied()
                    .filter(|row| self.when(row) == when)
                    .collect();
                (when, group)
            })
            .filter(|(_, group)| !group.is_empty())
            .collect()
    }

    /// When an ask has to be decided: Later in a paused project, else by its day (an ask with
    /// no day yet is this week's, the server's default).
    #[must_use]
    pub(crate) fn when(&self, row: &WorkRow) -> When {
        if row.paused {
            return When::Later;
        }
        lookup::decide_by(row).map_or(When::ThisWeek, |day| {
            When::of(&emberhut_api::time::Day::new(day), self.today)
        })
    }

    /// How urgent an ask is, the most urgent least: by when (overdue first), then its day (no
    /// day last), then the cheapest to answer, then the newest asked.
    fn urgency(
        &self,
        row: &WorkRow,
    ) -> (usize, bool, Option<String>, u8, Reverse<Option<Timestamp>>) {
        let day = lookup::decide_by(row);
        let when = When::ALL
            .iter()
            .position(|&when| when == self.when(row))
            .unwrap_or(When::ALL.len());
        (
            when,
            day.is_none(),
            day.map(str::to_owned),
            kind_rank(row),
            Reverse(lookup::asked(row).cloned()),
        )
    }

    /// The open top-level tasks, gone ones out.
    #[must_use]
    pub(crate) fn open_tasks(&self) -> Vec<&'a Issue> {
        self.state
            .open_tasks()
            .filter(|issue| !self.is_gone(Some(issue.id)))
            .collect()
    }

    /// A task's key as shown: its own, else made from its id.
    #[must_use]
    pub(crate) fn key(&self, task: TaskId) -> String {
        match lookup::issue(self.state, task) {
            Some(issue) if !issue.key.is_empty() => issue.key.clone(),
            _ => format!("{TASK_KEY_PREFIX}{task}"),
        }
    }

    /// The ask a waiting task really stands for: its own, unless it has no question and its
    /// action line names another task by its key (the prefix and the task's number); then that
    /// one, while it still waits on the user, else `None` (nothing to do here).
    pub(crate) fn ask_of(&self, task: &Issue) -> Option<TaskId> {
        if task.decision.is_some() {
            return Some(task.id);
        }
        let other = task
            .action
            .as_deref()
            .and_then(first_task_ref)
            .filter(|&id| id != task.id)
            .and_then(|id| lookup::issue(self.state, id));
        match other {
            None => Some(task.id),
            Some(other) => (other.status == TaskStatus::Waiting
                && (other.decision.is_some() || other.action.is_some()))
            .then_some(other.id),
        }
    }

    /// Acted on here, or standing for an ask that was.
    fn is_gone(&self, task: Option<TaskId>) -> bool {
        let Some(task) = task else { return false };
        self.gone.contains(&task)
            || lookup::issue(self.state, task)
                .and_then(|issue| self.ask_of(issue))
                .is_some_and(|ask| self.gone.contains(&ask))
    }

    /// Waiting on the user for real: not acted on here, and not standing for an ask that no
    /// longer waits.
    pub(crate) fn waits_on_you(&self, task: Option<TaskId>) -> bool {
        let Some(id) = task else { return false };
        !self.is_gone(task)
            && lookup::issue(self.state, id).is_none_or(|issue| self.ask_of(issue).is_some())
    }

    /// Whether an ask waits as Later: its project is paused, or it has to be decided more than
    /// a month from today (the This month tier ends there).
    pub(crate) fn is_later(&self, row: &WorkRow) -> bool {
        self.when(row) == When::Later
    }

    /// A row whose task stands for another row's ask: that row answers it.
    fn is_copy(&self, row: &WorkRow, rows: &[&WorkRow]) -> bool {
        let Some(id) = row.task else { return false };
        let Some(issue) = lookup::issue(self.state, id) else {
            return false;
        };
        match self.ask_of(issue) {
            Some(ask) if ask != id => rows.iter().any(|other| other.task == Some(ask)),
            _ => false,
        }
    }
}

/// What an ask costs the user, cheapest first: a quick yes, a read and a pick, something only
/// they can do. A row that says nothing reads as a pick.
#[must_use]
pub(crate) fn kind_rank(row: &WorkRow) -> u8 {
    match row.waiting.as_ref().map(|waiting| &waiting.need) {
        Some(AskKind::Approve) => 0,
        Some(AskKind::Do) => 2,
        _ => 1,
    }
}

/// The first task a line names: the number after the first [`TASK_KEY_PREFIX`] in it.
fn first_task_ref(line: &str) -> Option<TaskId> {
    let at = line.find(TASK_KEY_PREFIX)?;
    let digits: String = line[at + TASK_KEY_PREFIX.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().map(TaskId::new)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, state, waiting};

    /// The day the tests' lists stand on.
    const TODAY: Date = jiff::civil::date(2026, 11, 25);

    fn example_project() -> serde_json::Value {
        json!({"id": 7, "name": "Example", "goal": null, "priority": 0, "kind": "ongoing",
            "deadline": null, "hard": false, "status": "active", "pausedUntil": null, "open": 0,
            "doneWeek": 0, "health": null, "next": null, "hoursLeft": 0.0})
    }

    fn asks(id: i64) -> serde_json::Value {
        json!({"id": id, "key": format!("T-{id}"), "title": format!("Task {id}"), "owner": "you",
               "status": "waiting", "goal": null, "project": null, "pid": null, "priority": 0,
               "due": null, "today": false, "decideBy": null,
               "decision": {"question": "Which?", "options": ["A", "B"]}, "action": null,
               "updated": "2026-01-01T00:00:00Z", "doneAt": null, "parent": null, "score": 0.0})
    }

    fn pointing(id: i64, at: &str) -> serde_json::Value {
        serde_json::to_value(issue(id, "waiting", json!({"action": at}))).unwrap()
    }

    fn row(task: i64) -> serde_json::Value {
        serde_json::to_value(waiting(task, json!({}))).unwrap()
    }

    fn task_ids(rows: &[&WorkRow]) -> Vec<i64> {
        rows.iter()
            .filter_map(|row| row.task.map(TaskId::get))
            .collect()
    }

    #[test]
    fn task_references_are_read_from_action_lines() {
        assert_eq!(
            first_task_ref(&format!("One yes on {TASK_KEY_PREFIX}7 for the setup")),
            Some(TaskId::new(7))
        );
        assert_eq!(
            first_task_ref(&format!("{TASK_KEY_PREFIX}12")),
            Some(TaskId::new(12))
        );
        assert_eq!(first_task_ref("nothing here"), None);
        assert_eq!(first_task_ref(&format!("{TASK_KEY_PREFIX}x")), None);
    }

    #[test]
    fn only_waiting_rows_need_you() {
        let mut running = waiting(2, json!({}));
        running.state = emberhut_api::work::WorkState::Running;
        let mut taskless = waiting(3, json!({}));
        taskless.task = None;
        let state = state(json!({"issues": [asks(1)], "working": [row(1), running, taskless]}));
        let gone = HashSet::new();
        assert_eq!(task_ids(&Needs::new(&state, &gone, TODAY).rows()), [1]);
    }

    #[test]
    fn a_task_standing_for_another_ask_is_a_copy() {
        let copy = pointing(1, &format!("One yes on {TASK_KEY_PREFIX}2 for both"));
        let state = state(json!({"issues": [copy, asks(2)], "working": [row(1), row(2)]}));
        let gone = HashSet::new();
        assert_eq!(task_ids(&Needs::new(&state, &gone, TODAY).rows()), [2]);
    }

    #[test]
    fn a_task_standing_for_an_answered_ask_waits_on_nothing() {
        let done = serde_json::to_value(issue(2, "done", json!({}))).unwrap();
        let waits = pointing(1, &format!("Waits on {TASK_KEY_PREFIX}2"));
        let state = state(json!({"issues": [waits, done], "working": [row(1)]}));
        let gone = HashSet::new();
        assert_eq!(
            Needs::new(&state, &gone, TODAY).rows(),
            Vec::<&WorkRow>::new()
        );
    }

    #[test]
    fn gone_tasks_and_the_tasks_standing_for_them_leave_the_lists() {
        let copy = pointing(1, &format!("One yes on {TASK_KEY_PREFIX}2"));
        let state =
            state(json!({"issues": [copy, asks(2), asks(3)], "working": [row(1), row(2), row(3)]}));
        let gone = HashSet::from([TaskId::new(2)]);
        let needs = Needs::new(&state, &gone, TODAY);
        assert_eq!(task_ids(&needs.rows()), [3]);
        let open: Vec<i64> = needs.open_tasks().iter().map(|i| i.id.get()).collect();
        assert_eq!(open, [3]);
    }

    #[test]
    fn the_count_leaves_paused_projects_out() {
        let paused = serde_json::to_value(waiting(2, json!({"paused": true}))).unwrap();
        let state = state(json!({"issues": [asks(1), asks(2)], "working": [row(1), paused]}));
        let gone = HashSet::new();
        let needs = Needs::new(&state, &gone, TODAY);
        assert_eq!(needs.rows().len(), 2);
        assert_eq!(needs.count(), 1);
    }

    #[test]
    fn asks_group_by_project_soonest_first_with_other_last() {
        let on = |task: i64, pid: Option<i64>, day: Option<&str>| {
            serde_json::to_value(waiting(task, json!({"pid": pid, "decideBy": day}))).unwrap()
        };
        let state = state(json!({
            "projects": [example_project()],
            "issues": [asks(1), asks(2), asks(3), asks(4)],
            "working": [on(1, None, Some("2026-12-01")), on(2, Some(7), Some("2026-12-05")),
                        on(3, Some(7), Some("2026-11-30")), on(4, Some(99), None)],
        }));
        let gone = HashSet::new();
        let groups = {
            let needs = Needs::new(&state, &gone, TODAY);
            needs.by_project_of(needs.rows())
        };
        let shape: Vec<(Option<ProjectId>, &str, Vec<i64>)> = groups
            .iter()
            .map(|g| (g.project, g.name.as_str(), task_ids(&g.rows)))
            .collect();
        assert_eq!(
            shape,
            [
                (Some(ProjectId::new(7)), "Example", vec![3, 2]),
                (None, "Other", vec![1, 4])
            ]
        );
    }

    #[test]
    fn later_asks_fold_on_their_own_soonest_first_and_count_nowhere() {
        let on = |task: i64, extra: serde_json::Value| {
            serde_json::to_value(waiting(task, extra)).unwrap()
        };
        let state = state(json!({
            "projects": [example_project()],
            "issues": (1..=7).map(asks).collect::<Vec<_>>(),
            "working": [
                on(1, json!({"pid": 7, "decideBy": "2026-12-01"})),
                on(2, json!({"pid": 7, "paused": true})),
                on(3, json!({"decideBy": "2027-02-01"})),
                // 30 days out is still this month; 31 is Later.
                on(4, json!({"decideBy": "2026-12-25"})),
                on(5, json!({"pid": 99, "decideBy": "2026-12-26"})),
                on(6, json!({"pid": 7, "paused": true, "decideBy": "2026-12-02"})),
                on(7, json!({})),
            ],
        }));
        let gone = HashSet::new();
        let needs = Needs::new(&state, &gone, TODAY);

        let later = needs.later_of(needs.rows());
        assert_eq!(task_ids(&later), [6, 5, 3, 2]);
        let named: Vec<Option<&str>> = later
            .iter()
            .map(|row| lookup::project_name(&state, row.pid))
            .collect();
        assert_eq!(named, [Some("Example"), None, None, Some("Example")]);

        let grouped: Vec<Vec<i64>> = needs
            .by_project_of(needs.rows())
            .iter()
            .map(|group| task_ids(&group.rows))
            .collect();
        // Other: 7 has no day (this week's), 4 is this month's.
        assert_eq!(grouped, [vec![1], vec![7, 4]]);
        assert_eq!(
            needs.count(),
            3,
            "the groups add up to the count; Later is out"
        );
        assert_eq!(needs.rows().len(), 7, "Later still waits on the user");
    }

    #[test]
    fn open_tasks_are_top_level_and_not_finished() {
        let tasks: Vec<_> = [
            issue(1, "todo", json!({})),
            issue(2, "done", json!({})),
            issue(3, "canceled", json!({})),
            issue(4, "todo", json!({"parent": 1})),
        ]
        .into_iter()
        .map(|issue| serde_json::to_value(issue).unwrap())
        .collect();
        let state = state(json!({"issues": tasks}));
        let gone = HashSet::new();
        let open: Vec<i64> = Needs::new(&state, &gone, TODAY)
            .open_tasks()
            .iter()
            .map(|i| i.id.get())
            .collect();
        assert_eq!(open, [1]);
    }

    #[test]
    fn keys_come_from_the_task_or_its_id() {
        let todo = serde_json::to_value(issue(1, "todo", json!({}))).unwrap();
        let state = state(json!({"issues": [todo]}));
        let gone = HashSet::new();
        let needs = Needs::new(&state, &gone, TODAY);
        assert_eq!(needs.key(TaskId::new(1)), "T-1");
        assert_eq!(needs.key(TaskId::new(5)), format!("{TASK_KEY_PREFIX}5"));
    }
}
