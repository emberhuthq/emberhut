//! What the room switcher shows of each room's project and of the asks waiting there:
//!
//! - a project's room takes its project's name, goal, status and day to come back; it is active
//!   only while its project is; Main and the other rooms are active;
//! - a paused or archived room shows nothing unread and nothing waiting;
//! - the asks waiting in a room are the top-level tasks waiting on the person with something for
//!   them (an action or a question), whose project is the room's (none for Main and the rooms of
//!   no project, which also take an archived project's asks), and not Later (to decide more than
//!   a month out); the newest ask first. `ask` and `askId` are the first of them.

use std::collections::BTreeMap;

use emberhut_api::ids::ProjectId;
use emberhut_api::rooms::RoomRow;
use emberhut_api::tasks::{Issue, Project, ProjectStatus, TaskStatus};
use jiff::civil::Date;

use crate::services::personal_tasks::{decide_by_of, later};

/// Fills each row's project fields and asks from the board's `projects` and `issues`, on `today`.
/// `projects_of` names each row's project, by room id.
pub(crate) fn fill(
    rows: &mut [RoomRow],
    projects_of: &BTreeMap<i64, ProjectId>,
    projects: &[Project],
    issues: &[Issue],
    today: Date,
) {
    let by_id: BTreeMap<ProjectId, &Project> = projects.iter().map(|p| (p.id, p)).collect();
    let archived = |pid: Option<ProjectId>| {
        pid.and_then(|pid| by_id.get(&pid))
            .is_some_and(|p| p.status == ProjectStatus::Archived)
    };
    let mut waiting: Vec<&Issue> = issues
        .iter()
        .filter(|i| {
            i.status == TaskStatus::Waiting
                && i.parent.is_none()
                && (i.action.is_some() || i.decision.is_some())
                && !decide_by_of(i).is_some_and(|(day, _)| later(day, today))
        })
        .collect();
    waiting.sort_by(|a, b| {
        asked_at(b)
            .cmp(asked_at(a))
            .then_with(|| b.id.get().cmp(&a.id.get()))
    });
    for row in rows {
        let project = projects_of
            .get(&row.id.get())
            .and_then(|pid| by_id.get(pid));
        if let Some(project) = project {
            row.project = Some(project.id);
            row.name.clone_from(&project.name);
            row.goal = project.goal;
            row.status = project.status.clone();
            row.active = project.status == ProjectStatus::Active;
            row.paused_until.clone_from(&project.paused_until);
        }
        let asleep = matches!(row.status, ProjectStatus::Paused | ProjectStatus::Archived);
        if asleep {
            row.unread = 0;
            continue;
        }
        let home = row.project;
        let asks: Vec<&&Issue> = waiting
            .iter()
            .filter(|i| {
                let pid = if archived(i.pid) { None } else { i.pid };
                pid == home
            })
            .collect();
        row.needs = u32::try_from(asks.len()).unwrap_or(u32::MAX);
        row.ask = asks.first().map(|i| ask_line(i));
        row.ask_id = asks.first().map(|i| i.id);
    }
}

/// When it asked: its question's time, else its last change, as written.
fn asked_at(issue: &Issue) -> &str {
    issue
        .decision
        .as_ref()
        .and_then(|d| d.asked.as_ref())
        .unwrap_or(&issue.updated)
        .as_str()
}

/// The line a room's ask card shows: what to do, else the question, else the title.
fn ask_line(issue: &Issue) -> String {
    issue
        .action
        .clone()
        .or_else(|| {
            issue
                .decision
                .as_ref()
                .map(|d| d.question.clone())
                .filter(|q| !q.is_empty())
        })
        .unwrap_or_else(|| issue.title.clone())
}

#[cfg(test)]
mod tests {
    use emberhut_api::ids::{RoomId, TaskId};
    use emberhut_api::time::Day;

    use super::*;

    fn row(id: i64) -> RoomRow {
        RoomRow {
            id: RoomId::new(id),
            name: format!("Room {id}"),
            project: None,
            goal: None,
            active: true,
            status: ProjectStatus::Active,
            paused_until: None,
            unread: 3,
            needs: 0,
            ask: None,
            ask_id: None,
            busy: false,
            doing: None,
            last: None,
            last_at: None,
            last_mine: false,
        }
    }

    fn project(id: i64, name: &str, status: &ProjectStatus) -> Project {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": name, "goal": 4, "priority": 0, "kind": "deliverable",
            "deadline": null, "hard": false, "status": status.as_str(), "pausedUntil": null,
            "open": 0, "doneWeek": 0, "health": null, "next": null, "hoursLeft": 0.0
        }))
        .unwrap()
    }

    fn waiting(id: i64, pid: Option<i64>, extra: &serde_json::Value) -> Issue {
        let mut task = serde_json::json!({
            "id": id, "key": format!("K-{id}"), "title": format!("Task {id}"), "owner": "person",
            "status": "waiting", "goal": null, "project": null, "pid": pid, "priority": 3,
            "due": null, "today": false, "decision": null, "action": null,
            "updated": format!("2026-11-2{id}T10:00:00.000Z"), "doneAt": null
        });
        if let (Some(task), Some(extra)) = (task.as_object_mut(), extra.as_object()) {
            task.extend(extra.clone());
        }
        serde_json::from_value(task).unwrap()
    }

    #[test]
    fn a_project_room_shows_its_project_and_its_newest_ask_first() {
        let mut rows = vec![row(1), row(2), row(3), row(4)];
        let projects_of = BTreeMap::from([
            (2, ProjectId::new(10)),
            (3, ProjectId::new(11)),
            (4, ProjectId::new(12)),
        ]);
        let projects = [
            project(10, "Garden", &ProjectStatus::Active),
            project(11, "Old", &ProjectStatus::Archived),
            project(12, "Resting", &ProjectStatus::Paused),
        ];
        let later = Day::new("2027-06-01");
        let issues = [
            waiting(1, Some(10), &serde_json::json!({ "action": "Pick a spot" })),
            waiting(
                2,
                Some(10),
                &serde_json::json!({ "decision": { "question": "Tulips?", "options": ["Yes"] } }),
            ),
            waiting(
                3,
                Some(11),
                &serde_json::json!({ "action": "From an archived project" }),
            ),
            waiting(4, Some(12), &serde_json::json!({ "action": "Paused" })),
            waiting(
                5,
                None,
                &serde_json::json!({ "action": "Later", "decideBy": later }),
            ),
            waiting(6, Some(10), &serde_json::json!({})),
        ];
        let today = Date::constant(2026, 11, 26);
        fill(&mut rows, &projects_of, &projects, &issues, today);

        let main = &rows[0];
        assert_eq!(
            (main.needs, main.ask_id),
            (1, Some(TaskId::new(3))),
            "an archived project's ask is Main's"
        );
        assert!(main.active);
        let garden = &rows[1];
        assert_eq!(garden.project, Some(ProjectId::new(10)));
        assert_eq!(garden.name, "Garden");
        assert_eq!(garden.needs, 2);
        assert_eq!(
            garden.ask.as_deref(),
            Some("Tulips?"),
            "the newest ask first"
        );
        assert_eq!(garden.ask_id, Some(TaskId::new(2)));
        let old = &rows[2];
        assert!(!old.active);
        assert_eq!(old.status, ProjectStatus::Archived);
        assert_eq!((old.unread, old.needs), (0, 0));
        assert_eq!((rows[3].unread, rows[3].needs), (0, 0), "paused");
    }
}
