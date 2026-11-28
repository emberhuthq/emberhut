//! Every runner session works for a task (`--task-plans`): the person's own sessions get a task
//! made for them (opened from the app, or reported by a runner as theirs), with the plan asked
//! for in the first message the server sends; a session started by another works for its
//! parent's tasks; `thread_start` with no task makes one; sessions opened before the server
//! made tasks for them are left as they are. With the flag off, none of it happens.
#![expect(
    clippy::too_many_lines,
    reason = "each test walks one story through the endpoints from start to end"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_api::mcp::{CallToolResult, RpcResponse};
use emberhut_api::tasks::{Issue, TaskStatus};
use emberhut_api::wire::mcp::{ROOM_PATH, SESSION_HEADER};
use emberhut_core::RoomId;
use emberhut_core::task::session::{MADE_FOR_SESSION, opened_from};
use emberhut_core::task::{Owner, TaskId, Who};
use emberhut_db::NewTask;
use emberhut_server::app::Dependencies;
use emberhut_server::config::AuthMode;
use emberhut_server::services::tasks::session_rule;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};

/// The build runner's token.
const BUILD: &str = "token-of-build";

async fn app(task_plans: bool) -> TestApp {
    let app = TestApp::with_dependencies(AuthMode::Enforce, |dependencies, _| Dependencies {
        task_plans,
        ..dependencies
    })
    .await;
    app.runners.on(&[("build", "Build box", BUILD)]);
    app
}

async fn task(app: &TestApp, title: &str) -> TaskId {
    app.state
        .tasks()
        .create(
            NewTask {
                title: title.to_owned(),
                owner: Some(Owner::Assistant),
                ..NewTask::default()
            },
            Who::Assistant,
        )
        .await
        .unwrap()
        .id
}

async fn issue(app: &TestApp, id: TaskId) -> Issue {
    app.state.tasks().issue(id).await.unwrap().unwrap()
}

async fn history(app: &TestApp, id: TaskId) -> Vec<String> {
    app.state
        .tasks()
        .history_of(id)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.text)
        .collect()
}

async fn linked(app: &TestApp, session: &str) -> Vec<TaskId> {
    app.state.tasks().session_tasks(session).await.unwrap()
}

/// Every task there is, the newest last.
async fn all_tasks(app: &TestApp) -> Vec<TaskId> {
    let mut ids = Vec::new();
    for id in 1..50 {
        if app
            .state
            .tasks()
            .issue(TaskId::new(id))
            .await
            .unwrap()
            .is_some()
        {
            ids.push(TaskId::new(id));
        }
    }
    ids
}

/// A runner event of `kind` about `session`, its status `status` and its meta `meta`, opened at
/// `created` when given, `extra` added (a `turn`).
fn event(
    session: &str,
    kind: &str,
    status: &str,
    meta: Value,
    created: Option<Timestamp>,
    extra: Value,
) -> Value {
    let mut head = json!({ "id": session, "title": "Tidy", "project": "", "status": status });
    head["meta"] = meta;
    if let Some(created) = created {
        head["created"] = json!(created.to_string());
    }
    let mut event = json!({ "machine": "build", "kind": kind, "session": head });
    if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
        event.extend(extra);
    }
    event
}

/// Reports `event` as the build runner; it is taken.
async fn report(app: &TestApp, event: &Value) {
    let answer = app
        .call(
            Call::post("/api/runner/events")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(event),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
}

/// The person opens a session on the build runner with `text`.
async fn open(app: &TestApp, text: &str) -> crate::common::Answer {
    let session = app.sign_in().await;
    app.call(
        Call::post("/api/runner/sessions")
            .token(&session)
            .json(&json!({ "machine": "build", "cwd": "/work/app", "text": text })),
    )
    .await
}

/// A tool call as Main's session, with `session`'s header when given.
async fn as_room(
    app: &TestApp,
    session: Option<&str>,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let token = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    let body = json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    let mut request = Call::post(ROOM_PATH)
        .header("authorization", &format!("Bearer {}", token.as_str()))
        .json(&body);
    if let Some(session) = session {
        request = request.header(SESSION_HEADER, session);
    }
    let answer = app.call(request).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: RpcResponse = answer.read_as();
    serde_json::from_value(reply.result.unwrap()).unwrap()
}

#[tokio::test]
pub(crate) async fn the_persons_own_session_works_for_a_task_made_for_it() {
    let app = app(true).await;
    app.runners.answer(
        "build",
        "/sessions",
        json!({ "id": "r-1e8", "turn": "r-1e8:1", "cwd": "/work/app" }),
    );
    let message =
        "## Fix the **lexer** so it reads [raw strings](https://example.com/raw)\nAnd tests.";
    let answer = open(&app, message).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let made = *all_tasks(&app).await.last().unwrap();
    let row = issue(&app, made).await;
    assert_eq!(row.title, "Fix the lexer so it reads");
    assert_eq!(row.owner, "owner", "the person's");
    assert_eq!(row.status, TaskStatus::Working);
    assert_eq!(
        history(&app, made).await,
        ["Created".to_owned(), opened_from("Build box")]
    );

    let opened = app.runners.requests_to("/sessions")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(
        opened["meta"],
        json!({ "direct": true, "task": made.get() })
    );
    assert_eq!(
        opened["prompt"],
        format!("{message}\n\n{}", session_rule(&made.key())),
        "the first message asks for the plan"
    );

    // The runner reports it: it is linked, and its turn is the task's run.
    let meta = opened["meta"].clone();
    let prompt = opened["prompt"].as_str().unwrap();
    report(
        &app,
        &event(
            "r-1e8",
            "turn_queued",
            "queued",
            meta.clone(),
            Some(app.clock_now()),
            json!({ "turn": { "id": "r-1e8:1", "n": 1, "by": "owner", "prompt": prompt } }),
        ),
    )
    .await;
    report(
        &app,
        &event(
            "r-1e8",
            "turn_started",
            "running",
            meta,
            Some(app.clock_now()),
            json!({ "turn": { "id": "r-1e8:1", "n": 1 } }),
        ),
    )
    .await;
    assert_eq!(linked(&app, "r-1e8").await, [made]);
    let live = issue(&app, made).await.live.unwrap();
    assert_eq!(live.state.as_str(), "running");
    let session = app.sign_in().await;
    let mine: emberhut_api::runners::SessionsPage = app
        .call(
            Call::get("/api/runner/sessions")
                .query("direct=1")
                .token(&session),
        )
        .await
        .read_as();
    assert_eq!(
        mine.sessions[0].ask.as_deref(),
        Some("## Fix the **lexer** so it reads [raw strings](https://example.com/raw) And tests."),
        "the plan ask left out of what it was asked"
    );
    assert_eq!(mine.sessions[0].tasks.len(), 1);

    // A session the runner doesn't open leaves no task working.
    app.runners
        .refuse("build", "/sessions", "no room on this machine");
    let refused = open(&app, "Tidy the readme").await;
    assert_eq!(refused.status, 409, "{}", refused.text);
    let lost = *all_tasks(&app).await.last().unwrap();
    assert_ne!(lost, made);
    assert_eq!(issue(&app, lost).await.status, TaskStatus::Canceled);
    assert!(
        history(&app, lost)
            .await
            .last()
            .unwrap()
            .starts_with("Its session could not be started: "),
        "{:?}",
        history(&app, lost).await
    );
}

#[tokio::test]
pub(crate) async fn with_the_flag_off_no_session_gets_a_task() {
    let app = app(false).await;
    app.runners.answer(
        "build",
        "/sessions",
        json!({ "id": "r-1e8", "turn": "r-1e8:1" }),
    );
    let answer = open(&app, "Tidy the readme").await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let opened = app.runners.requests_to("/sessions")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(opened["meta"], json!({ "direct": true }));
    assert_eq!(opened["prompt"], "Tidy the readme");
    report(
        &app,
        &event(
            "r-2",
            "turn_queued",
            "queued",
            json!({ "direct": true }),
            Some(app.clock_now()),
            json!({ "turn": { "id": "r-2:1", "by": "owner", "prompt": "Look at it" } }),
        ),
    )
    .await;
    assert_eq!(all_tasks(&app).await.len(), 0, "no task made");

    let no_task = app
        .call_tool(
            "thread_start",
            json!({ "title": "Fix it", "message": "Go", "cwd": "/work/app" }),
        )
        .await;
    assert_eq!(
        text(&no_task),
        "Not started: pass task_id, the task the work is for."
    );
}

#[tokio::test]
pub(crate) async fn a_session_a_runner_reports_as_the_persons_gets_a_task_at_its_first_message() {
    let app = app(true).await;
    app.state.tasks().tick().await.unwrap();
    let now = app.clock_now();
    let direct = json!({ "direct": true });
    // Opened: no message yet, no task.
    report(
        &app,
        &event(
            "r-77",
            "session",
            "idle",
            direct.clone(),
            Some(now),
            json!({}),
        ),
    )
    .await;
    assert_eq!(all_tasks(&app).await.len(), 0);
    report(
        &app,
        &event(
            "r-77",
            "turn_queued",
            "queued",
            direct.clone(),
            Some(now),
            json!({ "turn": { "id": "r-77:1", "n": 1, "by": "owner",
                              "prompt": "> please *look* at the flaky test in ci" } }),
        ),
    )
    .await;
    let made = all_tasks(&app).await;
    assert_eq!(made.len(), 1);
    let row = issue(&app, made[0]).await;
    assert_eq!(row.title, "Please look at the flaky test");
    assert_eq!(
        (row.owner.as_str(), row.status),
        ("owner", TaskStatus::Working)
    );
    assert_eq!(linked(&app, "r-77").await, made);
    assert_eq!(
        issue(&app, made[0]).await.live.unwrap().state.as_str(),
        "queued"
    );
    // Its next turns stay on it; no second task.
    report(
        &app,
        &event(
            "r-77",
            "turn_queued",
            "queued",
            direct,
            Some(now),
            json!({ "turn": { "id": "r-77:2", "n": 2, "by": "owner", "prompt": "And another" } }),
        ),
    )
    .await;
    assert_eq!(all_tasks(&app).await, made);
}

#[tokio::test]
pub(crate) async fn sessions_opened_before_the_server_made_tasks_for_them_are_left_as_they_are() {
    let app = app(true).await;
    let direct = json!({ "direct": true });
    // A session the server knew before, which the runner reports with no opening time.
    report(
        &app,
        &event(
            "r-01d",
            "turn_finished",
            "idle",
            direct.clone(),
            None,
            json!({ "turn": { "id": "r-01d:3", "outcome": "completed", "last": "Done." } }),
        ),
    )
    .await;
    // The server starts making tasks for sessions now (its first tick, at start-up).
    app.state.tasks().tick().await.unwrap();
    let since = app.clock_now();
    app.clock.advance(SignedDuration::from_mins(5));
    report(
        &app,
        &event(
            "r-01d",
            "turn_queued",
            "queued",
            direct.clone(),
            None,
            json!({ "turn": { "id": "r-01d:4", "n": 4, "by": "owner", "prompt": "Carry on" } }),
        ),
    )
    .await;
    // One opened before then, first heard of now.
    let before = since - SignedDuration::from_hours(2);
    report(
        &app,
        &event(
            "r-0de",
            "turn_queued",
            "queued",
            direct.clone(),
            Some(before),
            json!({ "turn": { "id": "r-0de:9", "n": 9, "by": "owner", "prompt": "Carry on" } }),
        ),
    )
    .await;
    // A later tick doesn't move the line.
    app.state.tasks().tick().await.unwrap();
    assert_eq!(
        app.state.tasks().sessions_tasked_since().await.unwrap(),
        Some(since)
    );
    assert_eq!(all_tasks(&app).await.len(), 0, "old sessions get no task");

    // One opened after it does.
    report(
        &app,
        &event(
            "r-ae1",
            "turn_queued",
            "queued",
            direct,
            Some(since + SignedDuration::from_mins(1)),
            json!({ "turn": { "id": "r-ae1:1", "n": 1, "by": "owner", "prompt": "Carry on" } }),
        ),
    )
    .await;
    let made = all_tasks(&app).await;
    assert_eq!(made.len(), 1);
    assert_eq!(linked(&app, "r-ae1").await, made);
    assert_eq!(linked(&app, "r-01d").await, []);
    assert_eq!(linked(&app, "r-0de").await, []);
}

#[tokio::test]
pub(crate) async fn a_session_started_by_another_works_for_its_parents_tasks() {
    let app = app(true).await;
    let lexer = task(&app, "The lexer").await;
    let now = app.clock_now();
    report(
        &app,
        &event(
            "r-1",
            "turn_started",
            "running",
            json!({ "task": lexer.get() }),
            Some(now),
            json!({ "turn": { "id": "r-1:1" } }),
        ),
    )
    .await;
    assert_eq!(linked(&app, "r-1").await, [lexer]);
    // A session it started through the runner directly, naming it as its parent.
    report(
        &app,
        &event(
            "r-2",
            "session",
            "idle",
            json!({ "parent": "r-1" }),
            Some(now),
            json!({}),
        ),
    )
    .await;
    assert_eq!(linked(&app, "r-2").await, [lexer], "linked when first seen");
    report(
        &app,
        &event(
            "r-2",
            "turn_started",
            "running",
            json!({ "parent": "r-1" }),
            Some(now),
            json!({ "turn": { "id": "r-2:1" } }),
        ),
    )
    .await;
    let page = app.state.tasks().detail(lexer).await.unwrap().unwrap();
    let threads: Vec<&str> = page
        .runs
        .as_ref()
        .unwrap()
        .iter()
        .map(|run| run.thread.as_str())
        .collect();
    assert!(
        threads.contains(&"r-2"),
        "its turn is a run of its parent's task: {threads:?}"
    );

    // thread_start from a session (its header) with no task: the parent's task.
    app.runners.answer(
        "build",
        "/sessions",
        json!({ "id": "r-3", "turn": "r-3:1", "cwd": "/work/app" }),
    );
    let started = as_room(
        &app,
        Some("r-1"),
        "thread_start",
        json!({ "title": "The lexer's tests", "message": "Write them.", "cwd": "/work/app" }),
    )
    .await;
    assert!(
        text(&started).ends_with(&format!("It works for {}, its parent's task.", lexer.key())),
        "{}",
        text(&started)
    );
    let opened = app.runners.requests_to("/sessions")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(opened["meta"], json!({ "parent": "r-1" }));
    assert_eq!(
        opened["prompt"],
        format!("Write them.\n\n{}", session_rule(&lexer.key()))
    );
    // Named outright, the parent is carried too.
    let named = as_room(
        &app,
        None,
        "thread_start",
        json!({ "task_id": lexer.get(), "parent": "r-2", "title": "More", "message": "Go",
                "cwd": "/work/app" }),
    )
    .await;
    assert!(
        text(&named).starts_with("Session r-3 started"),
        "{}",
        text(&named)
    );
    let opened = app.runners.requests_to("/sessions")[1]
        .body
        .clone()
        .unwrap();
    assert_eq!(
        opened["meta"],
        json!({ "task": lexer.get(), "parent": "r-2" })
    );
    assert_eq!(all_tasks(&app).await, [lexer], "no task made");
}

#[tokio::test]
pub(crate) async fn a_session_the_assistant_starts_with_no_task_gets_one() {
    let app = app(true).await;
    app.runners.answer(
        "build",
        "/sessions",
        json!({ "id": "r-77", "turn": "r-77:1", "cwd": "/work/app" }),
    );
    let started = app
        .call_tool(
            "thread_start",
            json!({ "title": "Fix the build on main", "message": "The build fails.",
                    "cwd": "/work/app" }),
        )
        .await;
    let made = all_tasks(&app).await;
    assert_eq!(made.len(), 1);
    let made = made[0];
    assert!(
        text(&started).ends_with(&format!("It works for {}, a task made for it.", made.key())),
        "{}",
        text(&started)
    );
    let row = issue(&app, made).await;
    assert_eq!(row.title, "Fix the build on main");
    assert_eq!(
        (row.owner.as_str(), row.status),
        ("assistant", TaskStatus::Working)
    );
    assert_eq!(
        history(&app, made).await,
        ["Created by the assistant", MADE_FOR_SESSION]
    );
    let page = app.state.tasks().detail(made).await.unwrap().unwrap();
    assert_eq!(page.task.body, None, "no brief: a worker never takes it");
    let opened = app.runners.requests_to("/sessions")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(opened["meta"], json!({ "task": made.get() }));
    assert_eq!(
        opened["prompt"],
        format!("The build fails.\n\n{}", session_rule(&made.key()))
    );

    // A session that doesn't start leaves its task put away.
    app.runners.refuse("build", "/sessions", "no room");
    let refused = app
        .call_tool(
            "thread_start",
            json!({ "title": "Again", "message": "Go", "cwd": "/work/app" }),
        )
        .await;
    assert!(
        text(&refused).starts_with("Not started: "),
        "{}",
        text(&refused)
    );
    let lost = *all_tasks(&app).await.last().unwrap();
    assert_eq!(issue(&app, lost).await.status, TaskStatus::Canceled);
}
