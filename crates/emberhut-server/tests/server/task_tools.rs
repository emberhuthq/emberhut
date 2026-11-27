//! The tasks' and the plan's MCP tools, called through the endpoint as a room's agent session.
#![expect(
    clippy::too_many_lines,
    reason = "each test walks one story through the routes from start to end"
)]

use emberhut_core::task::TaskId;
use emberhut_core::task::wire::KEY_PREFIX;
use serde_json::{Value, json};

use crate::common::{TestApp, text};

async fn say(app: &TestApp, name: &str, arguments: Value) -> String {
    let result = app.call_tool(name, arguments).await;
    assert!(result.is_error != Some(true), "{name}: {}", text(&result));
    text(&result).to_owned()
}

async fn refused(app: &TestApp, name: &str, arguments: Value) -> String {
    let result = app.call_tool(name, arguments).await;
    assert!(
        result.is_error == Some(true),
        "{name} should refuse: {}",
        text(&result)
    );
    text(&result).to_owned()
}

#[tokio::test]
async fn tasks_are_made_changed_noted_and_finished_with_the_tools() {
    let app = TestApp::new().await;
    let project = say(
        &app,
        "project_set",
        json!({ "name": "Example", "priority": 2, "deadline": "2020-01-15", "hard": true }),
    )
    .await;
    assert_eq!(project, "Project #1.");
    let made = say(
        &app,
        "task_create",
        json!({ "title": "Write the report", "owner": "assistant", "project_id": 1, "body": "The brief", "size": "M" }),
    )
    .await;
    assert_eq!(made, format!("{KEY_PREFIX}1 created."));
    assert!(
        refused(
            &app,
            "task_create",
            json!({ "title": "x", "owner": "someone" })
        )
        .await
        .contains("unknown variant")
    );
    let waiting = say(
        &app,
        "task_create",
        json!({ "title": "x", "owner": "owner", "status": "waiting" }),
    )
    .await;
    assert!(
        waiting.starts_with("Not created: waiting needs an action"),
        "{waiting}"
    );

    let updated = say(
        &app,
        "task_update",
        json!({ "id": 1, "action": "Approve the outline", "decide_by": "2026-12-01", "link_threads": [{ "machine": "runner", "thread": "r-1" }] }),
    )
    .await;
    assert_eq!(
        updated,
        format!("{KEY_PREFIX}1: waiting (the user: Approve the outline).")
    );
    let back = say(&app, "task_update", json!({ "id": 1, "action": null })).await;
    assert_eq!(
        back,
        format!("{KEY_PREFIX}1: working."),
        "a cleared action sends the assistant's task back to work"
    );
    assert!(
        refused(&app, "task_update", json!({ "id": 1, "status": "waiting" }))
            .await
            .starts_with("waiting means it needs the person"),
    );
    assert_eq!(
        refused(&app, "task_update", json!({ "id": 9, "title": "x" })).await,
        format!("No task {KEY_PREFIX}9")
    );
    assert_eq!(
        say(
            &app,
            "task_note",
            json!({ "id": 1, "text": "Wrote the outline" })
        )
        .await,
        "Noted."
    );

    let one: Value = serde_json::from_str(&say(&app, "task", json!({ "id": 1 })).await).unwrap();
    assert_eq!(one["title"], "Write the report");
    assert_eq!(
        one["history"].as_array().unwrap().last().unwrap()["text"],
        "Wrote the outline"
    );
    assert_eq!(one["links"][0]["ref"], "r-1");

    let sub = say(
        &app,
        "task_create",
        json!({ "title": "Part one", "owner": "assistant", "parent_id": 1, "total": 3 }),
    )
    .await;
    assert_eq!(sub, format!("{KEY_PREFIX}2 created."));
    let progress: Value = serde_json::from_str(
        &say(&app, "task_progress", json!({ "parent_id": 1, "items": [{ "id": 2, "progress": 3 }, { "title": "Part two", "total": 2 }] })).await,
    )
    .unwrap();
    assert_eq!(
        progress["changed"],
        json!([format!("{KEY_PREFIX}2"), format!("{KEY_PREFIX}3")])
    );
    assert_eq!(progress["rollup"]["done"], 1);

    let overview = say(&app, "tasks", json!({})).await;
    assert!(
        overview.contains("Project 1 Example (active, late, ends 2020-01-15 hard)"),
        "{overview}"
    );
    assert!(
        overview.contains(&format!("~ {KEY_PREFIX}1 Write the report")),
        "{overview}"
    );
    let filtered = say(
        &app,
        "tasks",
        json!({ "status": ["done"], "owner": "assistant" }),
    )
    .await;
    assert!(
        filtered.contains("Filtered: status done, owner assistant"),
        "{filtered}"
    );
    let full: Value =
        serde_json::from_str(&say(&app, "tasks", json!({ "full": true })).await).unwrap();
    assert_eq!(full["tasks"][0]["subtasks"].as_array().unwrap().len(), 2);
    let projects: Value = serde_json::from_str(&say(&app, "projects", json!({})).await).unwrap();
    assert_eq!(projects[0]["name"], "Example");
    assert_eq!(projects[0]["health"], "late");

    assert_eq!(
        say(
            &app,
            "task_done",
            json!({ "id": 3, "summary": "Both parts are in" })
        )
        .await,
        "Done."
    );
    let worker = say(&app, "task_work", json!({ "id": 1 })).await;
    assert_eq!(worker, "Worker started.");
    assert_eq!(say(&app, "task_stop", json!({ "id": 1 })).await, "Stopped.");
    let parent = app
        .state
        .tasks()
        .issue(TaskId::new(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        parent.status.as_str(),
        "done",
        "every subtask done: the parent is done"
    );
}

/// `task_work` starts a worker only on the assistant's task with a brief; `task_stop` stops
/// it, closes its run and sends a task still marked working back to the assistant's to do.
#[tokio::test]
async fn a_worker_is_started_and_stopped_with_the_tools() {
    let app = TestApp::new().await;
    say(
        &app,
        "task_create",
        json!({ "title": "No brief", "owner": "assistant" }),
    )
    .await;
    say(
        &app,
        "task_create",
        json!({ "title": "Write it", "owner": "assistant", "body": "The brief" }),
    )
    .await;
    let not = say(&app, "task_work", json!({ "id": 1 })).await;
    assert_eq!(
        not,
        format!("Not started: {KEY_PREFIX}1 needs a brief (a body) and to be the assistant's")
    );
    assert_eq!(
        say(&app, "task_work", json!({ "id": 2 })).await,
        "Worker started."
    );
    assert_eq!(*app.task_workers.started.lock().unwrap(), vec![2]);

    // The workers' area starts the session and says so; the task shows it working.
    let tasks = app.state.tasks();
    tasks
        .worker_started(TaskId::new(2), "worker-2", "The brief")
        .await
        .unwrap();
    say(&app, "task_update", json!({ "id": 2, "status": "working" })).await;
    let running = tasks.issue(TaskId::new(2)).await.unwrap().unwrap();
    assert_eq!(running.status.as_str(), "working");
    let live = running.live.unwrap();
    assert_eq!(live.state.as_str(), "running");
    assert!(live.run.unwrap().worker);

    assert_eq!(say(&app, "task_stop", json!({ "id": 2 })).await, "Stopped.");
    assert_eq!(*app.task_workers.stopped.lock().unwrap(), vec![2]);
    let stopped = tasks.issue(TaskId::new(2)).await.unwrap().unwrap();
    assert_eq!(stopped.status.as_str(), "todo", "back to to do");
    assert_eq!(stopped.owner.as_str(), "assistant", "still the assistant's");
    assert_eq!(
        stopped.live.unwrap().state.as_str(),
        "todo",
        "no worker on it: it waits for one"
    );
    let detail = tasks.detail(TaskId::new(2)).await.unwrap().unwrap();
    assert_eq!(detail.runs.as_deref().unwrap_or_default().len(), 1);
    assert_eq!(detail.runs.as_ref().unwrap()[0].kind, "worker");
    assert_eq!(
        detail.runs.as_ref().unwrap()[0].outcome.as_deref(),
        Some("stopped")
    );
    assert!(
        detail.runs.as_ref().unwrap()[0].ended.is_some(),
        "its run is closed"
    );

    // A task with no worker: nothing to stop, and a done task stays done.
    say(&app, "task_done", json!({ "id": 2, "summary": "Written" })).await;
    assert_eq!(say(&app, "task_stop", json!({ "id": 2 })).await, "Stopped.");
    assert_eq!(*app.task_workers.stopped.lock().unwrap(), vec![2]);
    let done = tasks.issue(TaskId::new(2)).await.unwrap().unwrap();
    assert_eq!(done.status.as_str(), "done");
}

/// The ask options: cards to read and compare (one of them the pick, with why), and plain
/// answers beside them.
#[tokio::test]
async fn an_ask_lays_out_its_cards_and_its_pick() {
    let app = TestApp::new().await;
    say(
        &app,
        "task_create",
        json!({ "title": "Pick a pump", "owner": "assistant" }),
    )
    .await;
    let asked = say(
        &app,
        "task_ask",
        json!({
            "id": 1,
            "action": "Pick the pump",
            "question": "Two pumps fit.",
            "context": "The order goes out Friday",
            "kind": "choose",
            "options": [
                { "label": "Make #1", "title": "The Pump", "summary": "Cheap", "fields": { "Cost": "3", "Noise": "low" }, "pick": true, "why": "Cheapest", "then": "It gets ordered" },
                { "label": "Make #2", "title": "The Valve", "fields": { "Cost": "5", "Noise": "high" } },
                "More ideas"
            ],
            "decide_by": "2026-12-01"
        }),
    )
    .await;
    assert_eq!(
        asked,
        "Waiting on the user: Pick the pump (decide by 2026-12-01)."
    );
    let issue = app
        .state
        .tasks()
        .issue(TaskId::new(1))
        .await
        .unwrap()
        .unwrap();
    let decision = issue.decision.unwrap();
    assert_eq!(decision.options, ["Make #1", "Make #2", "More ideas"]);
    assert_eq!(decision.recommend, Some(0));
    let cards = decision.choices.unwrap();
    assert_eq!(cards[0].why.as_deref(), Some("Cheapest"));
    assert_eq!(cards[1].why, None, "only the pick says why");
    assert_eq!(cards[0].fields.as_ref().unwrap()[0].key, "Cost");
    assert_eq!(cards[2].title, None);
    assert_eq!(decision.notes.unwrap()[0], "It gets ordered");
    assert_eq!(
        decision.context.as_deref(),
        Some("The order goes out Friday")
    );
    let again = say(
        &app,
        "task_ask",
        json!({ "id": 1, "action": "Pick the pump", "question": "Still two.", "options": ["Make #1", "Make #2", "More ideas"], "same": true }),
    )
    .await;
    assert!(again.starts_with("Waiting on the user"), "{again}");
    let kept = app
        .state
        .tasks()
        .issue(TaskId::new(1))
        .await
        .unwrap()
        .unwrap()
        .decision
        .unwrap();
    assert!(
        kept.choices.is_some(),
        "a re-check with the same labels keeps the cards"
    );
    assert_eq!(kept.asked, decision.asked, "and when it was first asked");
    let two_picks = refused(
        &app,
        "task_ask",
        json!({ "id": 1, "action": "x", "question": "x", "options": [{ "label": "a", "pick": true }, { "label": "b", "pick": true }] }),
    )
    .await;
    assert_eq!(
        two_picks,
        "one pick at most: mark only the option you advise"
    );

    let list: emberhut_api::mcp::ToolsList = serde_json::from_value(
        app.rpc("tools/list", json!({}))
            .await
            .read_as::<emberhut_api::mcp::RpcResponse>()
            .result
            .unwrap(),
    )
    .unwrap();
    let ask = list.tools.iter().find(|t| t.name == "task_ask").unwrap();
    let option = &ask.input_schema["$defs"]["AskOption"];
    assert_eq!(
        option["oneOf"].as_array().map(Vec::len),
        Some(2),
        "an AskOption is a plain answer or a card: {option}"
    );
}

#[tokio::test]
async fn areas_hold_a_projects_work_as_trees() {
    let app = TestApp::new().await;
    say(&app, "project_set", json!({ "name": "Harness" })).await;
    let area = say(
        &app,
        "area_set",
        json!({ "project_id": 1, "name": "Checks", "goal": "Every check green" }),
    )
    .await;
    assert_eq!(area, "Area 1 \"Checks\" in Harness (active): no tasks yet.");
    say(
        &app,
        "task_create",
        json!({ "title": "Fix the flaky one", "owner": "assistant", "area_id": 1 }),
    )
    .await;
    say(
        &app,
        "task_create",
        json!({ "title": "Its first step", "owner": "assistant", "parent_id": 1 }),
    )
    .await;
    let outline = say(&app, "tasks", json!({ "area": "checks", "project_id": 1 })).await;
    assert!(
        outline.contains("Area 1 \"Checks\" (active, not started)"),
        "{outline}"
    );
    assert!(outline.contains("  Goal: Every check green"), "{outline}");
    assert!(
        outline.contains(&format!("○ {KEY_PREFIX}2 Its first step")),
        "{outline}"
    );
    let closed = say(&app, "area_set", json!({ "id": 1, "status": "done" })).await;
    assert!(
        closed.ends_with("It still has 1 open: move or finish them."),
        "{closed}"
    );
    assert_eq!(
        say(&app, "tasks", json!({ "area": "nowhere" })).await,
        "No area \"nowhere\"."
    );
    assert_eq!(
        refused(&app, "area_set", json!({ "name": "Loose" })).await,
        "a new area needs its project (project_id)"
    );
}

#[tokio::test]
async fn goals_focus_loops_and_the_concern_make_the_plan() {
    let app = TestApp::new().await;
    assert_eq!(
        say(
            &app,
            "goal_set",
            json!({ "title": "Health", "why": "Feel well" })
        )
        .await,
        "Goal #1, rank 1."
    );
    assert_eq!(
        say(
            &app,
            "goal_set",
            json!({ "title": "Income", "why": "Security", "rank": 1, "short": "Income" })
        )
        .await,
        "Goal #2, rank 1."
    );
    assert_eq!(
        refused(&app, "goal_set", json!({ "title": "No why" })).await,
        "a new goal needs a title and a why"
    );
    assert_eq!(
        say(
            &app,
            "focus_set",
            json!({ "title": "The launch", "line": "Under review", "pinned": true })
        )
        .await,
        "Saved as #1."
    );
    let pinned = refused(&app, "focus_drop", json!({ "id": 1 })).await;
    assert!(pinned.contains("pinned by the person"), "{pinned}");
    assert_eq!(
        say(
            &app,
            "loop_set",
            json!({ "who": "A colleague", "what": "The numbers", "due": null })
        )
        .await,
        "Loop #1."
    );
    assert_eq!(
        say(
            &app,
            "concern_set",
            json!({ "title": "Freedom", "why": "Everything serves it" })
        )
        .await,
        "Saved."
    );
    let plan: Value = serde_json::from_str(&say(&app, "plan", json!({})).await).unwrap();
    assert_eq!(plan["goals"][0]["title"], "Income");
    assert_eq!(plan["goals"][1]["rank"], 2);
    assert_eq!(plan["projects"][0]["title"], "The launch");
    assert_eq!(
        say(&app, "focus_drop", json!({ "id": 1, "owner_asked": true })).await,
        "Dropped."
    );
    assert_eq!(say(&app, "close_loop", json!({ "id": 1 })).await, "Closed.");
    assert_eq!(say(&app, "goal_drop", json!({ "id": 2 })).await, "Removed.");
    let state = app
        .state
        .state()
        .state(emberhut_core::RoomId::MAIN)
        .await
        .unwrap();
    assert!(state.loops.is_empty() && state.focus.is_empty());
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.report.unwrap().concern.unwrap().title, "Freedom");
}
