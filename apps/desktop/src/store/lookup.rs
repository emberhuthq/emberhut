//! Lookups over a state the contract leaves to its readers: a task by id, a project's name, a
//! goal's rank, the day a waiting row has to be decided by and how many days off it is.

use emberhut_api::ids::{GoalId, ProjectId, TaskId};
use emberhut_api::state::State;
use emberhut_api::tasks::Issue;
use emberhut_api::time::{Day, Timestamp};
use emberhut_api::work::WorkRow;
use jiff::civil::Date;

pub(crate) use crate::wire::PERSON_OWNER;
use crate::wire::{ASSISTANT_OWNER, CREATED_BY, QUIET_STEPS};

/// Whether a step of a task's history only changes its state ([`QUIET_STEPS`], or a creation
/// with its creator's name after [`CREATED_BY`]): a task's page leaves such steps out of the
/// exchange over its reply box.
#[must_use]
pub(crate) fn is_quiet_step(text: &str) -> bool {
    QUIET_STEPS.contains(&text) || text.starts_with(CREATED_BY)
}

/// Whether the assistant owns `issue` (else the user does).
#[must_use]
pub(crate) fn is_assistants(issue: &Issue) -> bool {
    issue.owner == ASSISTANT_OWNER
}

/// A task by its id.
#[must_use]
pub(crate) fn issue(state: &State, id: TaskId) -> Option<&Issue> {
    state.issues.iter().find(|issue| issue.id == id)
}

/// A project's name.
#[must_use]
pub(crate) fn project_name(state: &State, id: Option<ProjectId>) -> Option<&str> {
    let id = id?;
    state
        .projects
        .iter()
        .find(|project| project.id == id)
        .map(|project| project.name.as_str())
}

/// A goal's rank (a room's colour comes from it).
#[must_use]
pub(crate) fn goal_rank(state: &State, id: Option<GoalId>) -> Option<i32> {
    let id = id?;
    state
        .goals
        .iter()
        .find(|goal| goal.id == id)
        .map(|goal| goal.rank)
}

/// Whether the sidebar has Tasks: the project screens are hidden (the server's default when it
/// sends no display switches).
#[must_use]
pub(crate) fn shows_tasks(state: &State) -> bool {
    state.ui.as_ref().is_none_or(|ui| ui.hide_projects)
}

/// Whether the server keeps plans: tasks are the work that runs, each with its plan, and the
/// sessions move out of the main navigation. Off on an older server, which never says.
#[must_use]
pub(crate) fn keeps_plans(state: &State) -> bool {
    state.ui.as_ref().is_some_and(|ui| ui.task_plans)
}

/// Whether the sidebar lists Tasks: the project screens are hidden, or tasks carry plans (then
/// Tasks is the main place).
#[must_use]
pub(crate) fn lists_tasks(state: &State) -> bool {
    shows_tasks(state) || keeps_plans(state)
}

/// The day a waiting row has to be decided by, `YYYY-MM-DD`.
#[must_use]
pub(crate) fn decide_by(row: &WorkRow) -> Option<&str> {
    let deadline = row.waiting.as_ref()?.deadline.as_ref()?;
    deadline.decide_by.as_ref().map(Day::as_str)
}

/// A contract day (`YYYY-MM-DD`, maybe with a time after it) as a date.
#[must_use]
pub(crate) fn civil_day(day: &str) -> Option<Date> {
    day.get(0..10)?.parse().ok()
}

/// Days from `today` to `day`: negative once it has passed.
#[must_use]
pub(crate) fn days_from(today: Date, day: Date) -> Option<i32> {
    today.until(day).ok().map(|span| span.get_days())
}

/// How many days from `today` a waiting row has to be decided by: negative when late.
#[must_use]
pub(crate) fn days_to_decide(row: &WorkRow, today: Date) -> Option<i32> {
    days_from(today, civil_day(decide_by(row)?)?)
}

/// When a waiting row asked.
#[must_use]
pub(crate) fn asked(row: &WorkRow) -> Option<&Timestamp> {
    row.waiting.as_ref()?.asked.as_ref()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{state, waiting};

    #[test]
    fn names_and_ranks_are_looked_up_by_id() {
        let state = state(json!({
            "projects": [{"id": 4, "name": "Example", "goal": null, "priority": 0, "kind": "ongoing",
                          "deadline": null, "hard": false, "status": "active", "pausedUntil": null,
                          "open": 0, "doneWeek": 0, "health": null, "next": null, "hoursLeft": 0.0}],
            "goals": [{"id": 9, "title": "A goal", "why": "", "rank": 2, "status": null, "short": null}],
        }));
        assert_eq!(
            project_name(&state, Some(ProjectId::new(4))),
            Some("Example")
        );
        assert_eq!(project_name(&state, Some(ProjectId::new(5))), None);
        assert_eq!(goal_rank(&state, Some(GoalId::new(9))), Some(2));
        assert_eq!(goal_rank(&state, None), None);
    }

    #[test]
    fn the_assistants_tasks_are_known_by_their_owner() {
        use crate::fixtures::issue;
        let assistants = issue(1, "todo", json!({"owner": ASSISTANT_OWNER}));
        assert!(is_assistants(&assistants));
        assert!(!is_assistants(&issue(2, "todo", json!({"owner": "you"}))));
    }

    #[test]
    fn tasks_show_while_the_project_screens_are_hidden() {
        assert!(shows_tasks(&state(json!({}))));
        assert!(!shows_tasks(&state(json!({"ui": {"hideProjects": false}}))));
        assert!(!keeps_plans(&state(json!({}))));
        let plans = state(json!({"ui": {"hideProjects": false, "taskPlans": true}}));
        assert!(keeps_plans(&plans) && lists_tasks(&plans) && !shows_tasks(&plans));
    }

    #[test]
    fn a_waiting_rows_days_are_read_through_its_groups() {
        let row = waiting(
            3,
            json!({"decideBy": "2026-12-01", "asked": "2026-11-01T00:00:00Z"}),
        );
        assert_eq!(decide_by(&row), Some("2026-12-01"));
        assert_eq!(
            asked(&row).map(Timestamp::as_str),
            Some("2026-11-01T00:00:00Z")
        );
        assert_eq!(decide_by(&waiting(4, json!({}))), None);
    }

    #[test]
    fn a_waiting_rows_decide_by_is_counted_in_days_from_today() {
        let today = jiff::civil::date(2026, 11, 25);
        let on = |day: &str| waiting(5, json!({"decideBy": day}));
        assert_eq!(days_to_decide(&on("2026-12-25"), today), Some(30));
        assert_eq!(days_to_decide(&on("2026-11-23T18:00:00Z"), today), Some(-2));
        assert_eq!(days_to_decide(&on("soon"), today), None);
        assert_eq!(days_to_decide(&waiting(6, json!({})), today), None);
    }
}
