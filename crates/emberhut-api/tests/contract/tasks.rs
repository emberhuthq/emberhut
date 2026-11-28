//! A task's page, the task search, a message on a task, the task actions and today's health.

use emberhut_api::health_today::HealthToday;
use emberhut_api::ids::{AgentId, ProjectId, RunId, TaskId};
use emberhut_api::state::State;
use emberhut_api::task_actions::{Edit, TaskAction, TaskChange};
use emberhut_api::task_live::LiveState;
use emberhut_api::task_message::{MessageTo, TaskMessageReply, TaskMessageRequest};
use emberhut_api::task_page::{LinkKind, TaskPage};
use emberhut_api::task_plan::StepState;
use emberhut_api::task_search::{FoundTasks, ProjectScope, TaskQuery};
use emberhut_api::tasks::{Landed, TaskSize, TaskStatus};
use emberhut_api::time::{Day, Timestamp};
use emberhut_api::ultra::{AgentKind, AgentState, UltraState};
use serde_json::json;

use crate::support::{assert_round_trip, fixture, from_value, read, to_value};

#[test]
fn a_task_page_reads_its_task_and_everything_spread_in() {
    let page: TaskPage = read("task_page.json");
    let task = &page.task;
    assert_eq!(task.id, TaskId::new(12));
    assert!(task.ultra);
    let live = task.live.as_ref().unwrap();
    assert_eq!(live.state, LiveState::Running);
    let run = live.run.as_ref().unwrap();
    assert!(run.worker && run.title.is_none());
    assert_eq!(run.place.as_deref(), Some("the example machine"));
    assert_eq!(
        live.last.as_ref().unwrap().line.as_deref(),
        Some("Drafted both names")
    );
    let sub = task.sub.as_ref().unwrap();
    assert_eq!((sub.done, sub.total, sub.of), (1, 2, Some(6)));
    assert_eq!(sub.waiting, [TaskId::new(13)]);
    assert_eq!(task.agents.unwrap().running, 1);
    assert!(
        task.deadline.is_none(),
        "the board's fields take their defaults"
    );

    assert_eq!(page.history.len(), 4);
    assert_eq!(page.history[2].who, "owner");
    assert_eq!(page.subtasks[0].parent, Some(TaskId::new(12)));
    let finding = &page.findings[0];
    assert!(finding.is_finding() && !finding.is_open());
    assert_eq!(finding.landed, Some(Landed::Partly));
    assert_eq!(finding.fixed_by, Some(TaskId::new(13)));
    assert_eq!(page.links[0].kind, LinkKind::Thread);
    assert_eq!(page.links[1].reference, "EX-1");

    let runs = page.runs.as_ref().unwrap();
    assert_eq!(runs[0].id, RunId::new(31));
    assert!(runs[0].ended.is_none());
    assert_eq!(
        runs[1].status.as_deref(),
        Some("STATUS: needs emberhut: pick a name")
    );
    assert_eq!(page.sessions.len(), 1);

    let ultra = page.ultra_run.as_ref().unwrap();
    assert_eq!(ultra.state, Some(UltraState::Running));
    assert!(ultra.is_live() && ultra.shows());
    assert_eq!(ultra.agents[1].id, AgentId::new(2));
    assert_eq!(ultra.agents[1].kind, Some(AgentKind::Verify));
    assert_eq!(ultra.agents[0].state, Some(AgentState::Done));
    let lead = ultra.lead.as_ref().unwrap();
    assert!(lead.running);
    assert_eq!(lead.doing.as_deref(), Some("Planning part 2"));
    assert!((ultra.plan.unwrap().agents - 3.0).abs() < f64::EPSILON);
}

#[test]
fn a_written_page_reads_back_the_same() {
    // The board's `deadline`, `hardDeadline` and `score` are written with their defaults, which
    // a page from the server leaves out: the page reads back as it was, not byte for byte.
    let page: TaskPage = read("task_page.json");
    let again: TaskPage = from_value(to_value(&page));
    assert_eq!(again, page);
}

#[test]
fn a_page_from_an_older_server_has_no_runs_or_agents() {
    let mut body = fixture("task_page.json");
    let object = body.as_object_mut().unwrap();
    for field in ["runs", "ultraRun", "live", "findings", "links"] {
        object.remove(field);
    }
    let page: TaskPage = from_value(body);
    assert!(page.runs.is_none() && page.ultra_run.is_none() && page.task.live.is_none());
    assert!(page.findings.is_empty() && page.links.is_empty());
}

#[test]
fn found_tasks_read_as_tasks() {
    let found: FoundTasks = read("tasks_found.json");
    assert_eq!(found.tasks.len(), 2);
    assert_eq!(found.tasks[0].status, TaskStatus::Working);
    assert_eq!(found.tasks[1].kind.as_deref(), Some("finding"));
}

#[test]
fn a_search_asks_with_the_servers_names() {
    let query = TaskQuery {
        limit: Some(50),
        project: Some(ProjectScope::Project(ProjectId::new(2))),
        statuses: vec![TaskStatus::Waiting],
        owner: Some("owner".to_owned()),
        ..TaskQuery::search("T-212")
    };
    assert_eq!(
        query.to_query(),
        "q=T-212&limit=50&project=2&status=waiting&owner=owner"
    );
}

#[test]
fn a_message_on_a_task_round_trips() {
    let request: TaskMessageRequest = assert_round_trip("task_message_request.json");
    assert_eq!(request.files.len(), 1);
    let reply: TaskMessageReply = assert_round_trip("task_message_reply.json");
    assert_eq!(reply.to, MessageTo::Worker);
    assert_eq!(reply.queued, Some(true));
    let to_the_assistant: TaskMessageReply =
        from_value(json!({"ok": true, "to": MessageTo::ASSISTANT}));
    assert_eq!(to_the_assistant.to, MessageTo::Assistant);
}

#[test]
fn the_task_actions_are_the_servers_bodies() {
    let task = TaskId::new(12);
    assert_eq!(
        to_value(&TaskAction::park(
            task,
            Timestamp::new("2026-12-02T00:00:00.000Z")
        )),
        json!({"todoPark": 12, "until": "2026-12-02T00:00:00.000Z"})
    );
    let change = TaskChange {
        deadline: Edit::Set(Day::new("2026-12-03")),
        hard: Some(true),
        size: Edit::Clear,
        ..TaskChange::of(task)
    };
    assert_eq!(
        to_value(&TaskAction::from(change)),
        json!({"taskSet": {"id": 12, "deadline": "2026-12-03", "hard": true, "size": null}})
    );
    assert_eq!(
        to_value(&TaskAction::from(TaskChange {
            size: Edit::Set(TaskSize::ExtraLarge),
            ultra: Some(false),
            ..TaskChange::of(task)
        })),
        json!({"taskSet": {"id": 12, "size": "XL", "ultra": false}})
    );
}

#[test]
fn the_state_reads_todays_health() {
    let mut body = fixture("state.json");
    body["health"] = json!({
        "date": "2026-11-25", "steps": 4200.0, "exerciseMin": 12.0, "moveMin": 18.0,
        "stepsTarget": 8000.0, "at": "2026-11-25T08:00:00.000Z", "project": 2,
        "tasks": [{"id": 13, "target": 30.0}]
    });
    let state: State = from_value(body);
    let health: &HealthToday = state.health.as_ref().unwrap();
    assert_eq!(health.walk_target(TaskId::new(13)), Some(30.0));
    assert_eq!(health.walk_target(TaskId::new(12)), None);
    assert_eq!(health.project, Some(ProjectId::new(2)));
}

#[test]
fn a_page_carries_its_task_s_plan_at_a_glance_and_in_full() {
    let mut body = fixture("task_page.json");
    let object = body.as_object_mut().unwrap();
    object.insert(
        "plan".to_owned(),
        json!({ "done": 1, "total": 3, "current": { "id": 2, "title": "Run the tests", "state": "doing" },
                "eta": "about 2 hours", "updated": "2026-11-28T06:50:17.291Z" }),
    );
    object.insert(
        "fullPlan".to_owned(),
        json!({
            "steps": [
                { "id": 1, "title": "Build the Mac app", "state": "done", "finished": "2026-11-28T06:40:00.000Z",
                  "updated": "2026-11-28T06:40:00.000Z" },
                { "id": 2, "title": "Run the tests", "state": "doing", "started": "2026-11-28T06:45:00.000Z",
                  "updated": "2026-11-28T06:45:00.000Z" },
                { "id": 3, "parent": 2, "title": "Unit tests", "state": "blocked", "note": "needs the key",
                  "updated": "2026-11-28T06:50:17.291Z" },
            ],
            "summary": { "done": 1, "total": 3, "eta": "about 2 hours" },
        }),
    );
    let page: TaskPage = from_value(body.clone());
    let glance = page.task.plan.as_ref().unwrap();
    assert_eq!((glance.done, glance.total), (1, 3));
    assert_eq!(glance.current.as_ref().unwrap().state, StepState::Doing);
    let plan = page.plan.as_ref().unwrap();
    assert_eq!(plan.steps[2].parent, Some(2));
    assert_eq!(plan.steps[2].state, StepState::Blocked);
    assert_eq!(plan.summary.eta.as_deref(), Some("about 2 hours"));
    let written = to_value(&page);
    assert_eq!(written["plan"], body["plan"], "the task's own: at a glance");
    assert_eq!(written["fullPlan"], body["fullPlan"]);

    let older: TaskPage = read("task_page.json");
    assert!(
        older.plan.is_none() && older.task.plan.is_none(),
        "no plans kept"
    );
    assert!(to_value(&older).get("fullPlan").is_none());
}
