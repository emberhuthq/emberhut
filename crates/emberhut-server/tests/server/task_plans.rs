//! Tasks' plans (`--task-plans`): the plan tools on the room and worker endpoints and their
//! scope (a worker's task, a runner session's task named by its session header), each task's
//! plan at a glance in the state and in full on its page, the history each change writes, a
//! turn that ends with no plan, a runner session's turns as its task's runs, and the session
//! header in the runners' shared MCP list. With the flag off, none of it shows.
#![expect(
    clippy::too_many_lines,
    reason = "each test walks one story through the endpoints from start to end"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_api::mcp::{CallToolResult, RpcResponse};
use emberhut_api::runners::SessionsPage;
use emberhut_api::state::State;
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_plan::StepState;
use emberhut_api::wire::mcp::{ROOM_PATH, SESSION_HEADER, SESSION_HEADER_VALUE};
use emberhut_core::RoomId;
use emberhut_core::task::{Owner, TaskId, Who};
use emberhut_db::NewTask;
use emberhut_server::app::Dependencies;
use emberhut_server::config::AuthMode;
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
                body: Some(format!("Do {title}.")),
                ..NewTask::default()
            },
            Who::Assistant,
        )
        .await
        .unwrap()
        .id
}

/// `tools/call` of `name` on `path` with the bearer `token` and, when given, the session
/// header: the answer's result.
async fn call(
    app: &TestApp,
    path: &str,
    token: &str,
    session: Option<&str>,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let body = json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    let mut request = Call::post(path)
        .header("authorization", &format!("Bearer {token}"))
        .json(&body);
    if let Some(session) = session {
        request = request.header(SESSION_HEADER, session);
    }
    let answer = app.call(request).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: RpcResponse = answer.read_as();
    serde_json::from_value(reply.result.unwrap()).unwrap()
}

/// A tool call as Main's session, with `session`'s header when given.
async fn as_room(
    app: &TestApp,
    session: Option<&str>,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let token = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    call(app, ROOM_PATH, token.as_str(), session, name, arguments).await
}

/// A tool call as `task`'s worker.
async fn as_worker(app: &TestApp, task: TaskId, name: &str, arguments: Value) -> CallToolResult {
    let token = app.state.worker_tokens().for_task(task).unwrap();
    call(app, "/mcp/worker", token.as_str(), None, name, arguments).await
}

async fn tool_names(app: &TestApp) -> Vec<String> {
    let answer = app.rpc("tools/list", json!({})).await;
    let reply: RpcResponse = answer.read_as();
    reply.result.unwrap()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn state(app: &TestApp) -> State {
    let session = app.sign_in().await;
    app.call(Call::get("/api/state").token(&session))
        .await
        .read_as()
}

async fn page(app: &TestApp, task: TaskId) -> TaskPage {
    let session = app.sign_in().await;
    let answer = app
        .call(Call::get(&format!("/api/tasks/{}", task.get())).token(&session))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as()
}

fn error(result: &CallToolResult) -> &str {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    text(result)
}

#[tokio::test]
pub(crate) async fn with_the_flag_off_no_plan_shows_and_the_tools_are_absent() {
    let app = app(false).await;
    let id = task(&app, "the parser review").await;
    // A plan written before the flag went off is kept, never shown.
    app.state
        .tasks()
        .set_plan(
            id,
            Vec::new(),
            Some(Some("soon".to_owned())),
            Who::Assistant,
        )
        .await
        .unwrap();
    let names = tool_names(&app).await;
    assert!(
        !names.iter().any(|name| name.starts_with("plan_")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|name| name == "plan"),
        "the weekly plan stays"
    );
    let state = state(&app).await;
    assert!(!state.ui.unwrap().task_plans);
    let issue = state
        .issues
        .iter()
        .find(|i| i.id.get() == id.get())
        .unwrap();
    assert_eq!(issue.plan, None);
    assert!(page(&app, id).await.plan.is_none());
    let worker = as_worker(&app, id, "plan_set", json!({ "steps": [] })).await;
    assert!(error(&worker).contains("plan_set"), "{worker:?}");
}

#[tokio::test]
pub(crate) async fn an_agent_writes_the_plan_ticks_it_and_the_apps_read_it() {
    let app = app(true).await;
    let id = task(&app, "the Mac app").await;
    let other = task(&app, "the website").await;
    assert!(
        state(&app).await.ui.unwrap().task_plans,
        "the flag, not a setting"
    );
    let fresh = state(&app).await;
    let row = fresh
        .issues
        .iter()
        .find(|i| i.id.get() == id.get())
        .unwrap();
    let empty = row.plan.as_ref().unwrap();
    assert!(empty.is_empty(), "no plan yet");

    let names = tool_names(&app).await;
    for name in ["plan_set", "plan_step", "plan_read"] {
        assert!(names.iter().any(|n| n == name), "{name}");
    }
    let unnamed = as_room(&app, None, "plan_set", json!({ "steps": [] })).await;
    assert!(error(&unnamed).starts_with("Say which task"));

    let set = as_room(
        &app,
        None,
        "plan_set",
        json!({
            "task": id.key(),
            "steps": [
                { "title": "Build the Mac app", "state": "doing", "steps": [
                    { "title": "Compile", "state": "done" },
                    { "title": "Strip the symbols" },
                ] },
                { "title": "Run the tests" },
                { "title": "Sign the build" },
            ],
            "eta": "about 2 hours",
        }),
    )
    .await;
    assert_ne!(set.is_error, Some(true), "{set:?}");
    assert_eq!(
        text(&set),
        format!(
            "Plan of {} \"the Mac app\":\n[>] 1 Build the Mac app\n  [x] 2 Compile\n  [ ] 3 Strip the symbols\n[ ] 4 Run the tests\n[ ] 5 Sign the build\n\n1 of 4 done; now: 3 Strip the symbols; estimate: about 2 hours",
            id.key()
        )
    );

    let blocked = as_room(
        &app,
        None,
        "plan_step",
        json!({ "task": id.get(), "step": "Sign the build", "state": "blocked", "note": "needs the certificate" }),
    )
    .await;
    assert_ne!(blocked.is_error, Some(true), "{blocked:?}");
    let ticked = as_room(
        &app,
        None,
        "plan_step",
        json!({ "task": id.get(), "step": 3, "state": "done" }),
    )
    .await;
    assert!(text(&ticked).contains("[x] 3 Strip the symbols"));
    let added = as_room(
        &app,
        None,
        "plan_step",
        json!({ "task": id.get(), "add": { "title": "Notarize", "after": 5 } }),
    )
    .await;
    assert!(text(&added).ends_with("[ ] 6 Notarize\n\n2 of 5 done; now: 5 Sign the build; blocked: needs the certificate; estimate: about 2 hours"), "{}", text(&added));
    let missing = as_room(
        &app,
        None,
        "plan_step",
        json!({ "task": id.get(), "step": 42, "state": "done" }),
    )
    .await;
    assert!(error(&missing).starts_with("no step 42"));
    let deep = as_room(
        &app,
        None,
        "plan_set",
        json!({ "task": id.get(), "steps": [{ "title": "A", "steps": [{ "title": "B", "steps": [] }] }] }),
    )
    .await;
    assert!(error(&deep).contains("unknown field `steps`"), "{deep:?}");

    // The task list shows it at a glance, the task's page in full.
    let state = state(&app).await;
    let row = state
        .issues
        .iter()
        .find(|i| i.id.get() == id.get())
        .unwrap();
    let glance = row.plan.as_ref().unwrap();
    assert_eq!((glance.done, glance.total), (2, 5));
    let current = glance.current.as_ref().unwrap();
    assert_eq!(
        (current.id, current.title.as_str()),
        (5, "Sign the build"),
        "none being worked on: the blocked one"
    );
    assert_eq!(glance.blocker.as_deref(), Some("needs the certificate"));
    assert_eq!(glance.eta.as_deref(), Some("about 2 hours"));
    assert!(glance.updated.is_some());
    let untouched = state
        .issues
        .iter()
        .find(|i| i.id.get() == other.get())
        .unwrap();
    assert!(untouched.plan.as_ref().unwrap().is_empty());
    let page = page(&app, id).await;
    let plan = page.plan.unwrap();
    assert_eq!(plan.summary, *glance);
    assert_eq!(page.task.plan.as_ref(), Some(glance));
    let ids: Vec<(u32, Option<u32>)> = plan.steps.iter().map(|s| (s.id, s.parent)).collect();
    assert_eq!(
        ids,
        [
            (1, None),
            (2, Some(1)),
            (3, Some(1)),
            (4, None),
            (5, None),
            (6, None)
        ]
    );
    assert_eq!(plan.steps[1].state, StepState::Done);
    assert!(plan.steps[1].finished.is_some());
    assert!(plan.steps[0].started.is_some());
    let lines: Vec<&str> = page.history.iter().map(|m| m.text.as_str()).collect();
    assert!(
        lines.ends_with(&[
            "Plan set: 4 steps, 1 done",
            "Blocked: Sign the build — needs the certificate",
            "Ticked: Strip the symbols",
            "Added step: Notarize",
        ]),
        "{lines:?}"
    );
    assert_eq!(page.task.last.as_deref(), Some("Added step: Notarize"));

    // Setting it again keeps the ticks of the steps it names again.
    let again = as_room(
        &app,
        None,
        "plan_set",
        json!({ "task": id.key(), "steps": [
            { "title": "build the mac app", "steps": [{ "title": "Compile" }] },
            { "title": "Ship it" },
        ] }),
    )
    .await;
    assert!(
        text(&again).starts_with(&format!(
            "Plan of {} \"the Mac app\":\n[>] 1 build the mac app\n  [x] 2 Compile\n[ ] 7 Ship it",
            id.key()
        )),
        "{}",
        text(&again)
    );
    let read = as_room(&app, None, "plan_read", json!({ "task": id.key() })).await;
    assert_eq!(text(&read), text(&again));
}

#[tokio::test]
pub(crate) async fn a_worker_and_a_runner_session_keep_their_own_tasks_plan() {
    let app = app(true).await;
    let mine = task(&app, "the parser").await;
    let other = task(&app, "the lexer").await;

    let worker = as_worker(
        &app,
        mine,
        "plan_set",
        json!({ "steps": [{ "title": "Read it" }] }),
    )
    .await;
    assert_ne!(worker.is_error, Some(true), "{worker:?}");
    let history = app.state.tasks().history_of(mine).await.unwrap();
    assert_eq!(history.last().unwrap().who, Who::Worker);
    let elsewhere = as_worker(
        &app,
        mine,
        "plan_step",
        json!({ "task": other.key(), "add": { "title": "Nope" } }),
    )
    .await;
    assert_eq!(
        error(&elsewhere),
        format!(
            "You work on {}: you may read {}'s plan but not write it.",
            mine.key(),
            other.key()
        )
    );
    let reads = as_worker(&app, mine, "plan_read", json!({ "task": other.get() })).await;
    assert!(text(&reads).contains("No plan yet"));

    // A runner session the assistant started for `other`: its events link it to the task.
    let session = "r-5eed";
    let reported = app
        .call(
            Call::post("/api/runner/events")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(&json!({
                    "machine": "build", "kind": "turn_started", "at": "2026-11-25T10:00:00.000Z",
                    "session": { "id": session, "title": "The lexer", "project": "", "status": "running",
                                 "meta": { "task": other.get() } },
                    "turn": { "id": "r-5eed:1", "started": "2026-11-25T10:00:00.000Z" },
                })),
        )
        .await;
    assert_eq!(reported.status, 200, "{}", reported.text);
    let own = as_room(
        &app,
        Some(session),
        "plan_set",
        json!({ "steps": [{ "title": "Tokens", "state": "doing" }] }),
    )
    .await;
    assert!(
        text(&own).starts_with(&format!("Plan of {} \"the lexer\"", other.key())),
        "the session's own task by default: {own:?}"
    );
    let not_its = as_room(
        &app,
        Some(session),
        "plan_step",
        json!({ "task": mine.get(), "step": 1, "state": "done" }),
    )
    .await;
    assert!(error(&not_its).starts_with(&format!("You work on {}", other.key())));
    // An unexpanded or unknown session is as no header: the room's own rights.
    for (unknown, state) in [
        (SESSION_HEADER_VALUE, "doing"),
        ("r-0ff", "done"),
        ("not a session", "todo"),
    ] {
        let room = as_room(
            &app,
            Some(unknown),
            "plan_step",
            json!({ "task": mine.get(), "step": 1, "state": state }),
        )
        .await;
        assert_ne!(room.is_error, Some(true), "{unknown}: {room:?}");
    }
    let unnamed = as_room(&app, Some("r-0ff"), "plan_read", json!({})).await;
    assert!(error(&unnamed).starts_with("Say which task"));
}

#[tokio::test]
pub(crate) async fn a_runner_session_working_on_a_task_shows_it_running() {
    let app = app(true).await;
    let id = task(&app, "the lexer").await;
    let event = |kind: &str, status: &str, extra: Value| {
        let mut event = json!({
            "machine": "build", "kind": kind, "at": "2026-11-25T10:00:00.000Z",
            "session": { "id": "r-1e8", "title": "The lexer", "project": "", "status": status,
                         "meta": { "task": id.key() } },
        });
        if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
            event.extend(extra);
        }
        event
    };
    let report = |body: Value| {
        let app = &app;
        async move {
            let answer = app
                .call(
                    Call::post("/api/runner/events")
                        .header("authorization", &format!("Bearer {BUILD}"))
                        .json(&body),
                )
                .await;
            assert_eq!(answer.status, 200, "{}", answer.text);
        }
    };
    let live = |state: &State| {
        state
            .issues
            .iter()
            .find(|i| i.id.get() == id.get())
            .unwrap()
            .live
            .clone()
            .unwrap()
    };
    report(event(
        "turn_queued",
        "queued",
        json!({ "turn": { "id": "r-1e8:1", "by": "assistant", "prompt": "Write the lexer" } }),
    ))
    .await;
    assert_eq!(live(&state(&app).await).state.as_str(), "queued");
    report(event(
        "turn_started",
        "running",
        json!({ "turn": { "id": "r-1e8:1" } }),
    ))
    .await;
    // The same event twice changes nothing.
    report(event(
        "turn_started",
        "running",
        json!({ "turn": { "id": "r-1e8:1" } }),
    ))
    .await;
    let running = live(&state(&app).await);
    assert_eq!(running.state.as_str(), "running");
    let run = running.run.unwrap();
    assert_eq!(
        (run.thread.as_str(), run.machine.as_str()),
        ("r-1e8", "build")
    );
    let page = page(&app, id).await;
    assert_eq!(page.runs.as_ref().unwrap().len(), 1, "one run for the turn");
    assert!(
        page.links.iter().any(|link| link.reference == "r-1e8"),
        "the session is linked to its task"
    );
    let session = app.sign_in().await;
    let listed: SessionsPage = app
        .call(
            Call::get("/api/runner/sessions")
                .query(&format!("task={}", id.get()))
                .token(&session),
        )
        .await
        .read_as();
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(
        listed.sessions[0].tasks,
        [emberhut_api::ids::TaskId::new(id.get())]
    );

    report(event(
        "turn_finished",
        "idle",
        json!({ "turn": { "id": "r-1e8:1", "outcome": "completed", "last": "Wrote it.\nSTATUS: done" } }),
    ))
    .await;
    let after = state(&app).await;
    assert_ne!(live(&after).state.as_str(), "running");
    let row = after
        .issues
        .iter()
        .find(|i| i.id.get() == id.get())
        .unwrap();
    assert!(
        row.plan.as_ref().unwrap().unplanned_turn.is_some(),
        "the turn ended with no plan"
    );
    report(event(
        "turn_started",
        "running",
        json!({ "turn": { "id": "r-1e8:2" } }),
    ))
    .await;
    report(event(
        "turn_finished",
        "idle",
        json!({ "turn": { "id": "r-1e8:2", "outcome": "completed", "last": "More." } }),
    ))
    .await;
    let history = app.state.tasks().history_of(id).await.unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|line| line.text == "A turn ended with no plan")
            .count(),
        1,
        "once for turns in a row"
    );
    assert_eq!(page_runs(&app, id).await, 2);
}

async fn page_runs(app: &TestApp, id: TaskId) -> usize {
    page(app, id).await.runs.map_or(0, |runs| runs.len())
}

/// A runner event of `kind` about the session `session` working on `task`, its status
/// `status`, `extra` added (a `turn`, an `at`).
fn session_event(task: TaskId, session: &str, kind: &str, status: &str, extra: Value) -> Value {
    let mut event = json!({
        "machine": "build", "kind": kind, "at": "2026-11-25T10:00:00.000Z",
        "session": { "id": session, "title": "The lexer", "project": "", "status": status,
                     "meta": { "task": task.key() } },
    });
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

/// The sessions list, of `task`'s sessions when given.
async fn listed(app: &TestApp, task: Option<TaskId>) -> SessionsPage {
    let session = app.sign_in().await;
    let mut request = Call::get("/api/runner/sessions").token(&session);
    if let Some(task) = task {
        request = request.query(&format!("task={}", task.get()));
    }
    app.call(request).await.read_as()
}

#[tokio::test]
pub(crate) async fn with_the_flag_off_a_sessions_task_is_not_followed() {
    let app = app(false).await;
    let id = task(&app, "the lexer").await;
    for (kind, status) in [
        ("turn_queued", "queued"),
        ("turn_started", "running"),
        ("activity", "running"),
    ] {
        report(
            &app,
            &session_event(
                id,
                "r-0ff1",
                kind,
                status,
                json!({ "turn": { "id": "r-0ff1:1", "by": "assistant", "prompt": "Write it" } }),
            ),
        )
        .await;
    }
    let state = state(&app).await;
    let issue = state
        .issues
        .iter()
        .find(|i| i.id.get() == id.get())
        .unwrap();
    assert!(
        issue.live.as_ref().is_none_or(|live| live.run.is_none()),
        "no run: {:?}",
        issue.live
    );
    assert_eq!(page_runs(&app, id).await, 0);
    assert!(
        !page(&app, id)
            .await
            .links
            .iter()
            .any(|link| link.reference == "r-0ff1"),
        "the session isn't linked"
    );
    let all = listed(&app, None).await;
    let row = all
        .sessions
        .iter()
        .find(|row| row.id.as_str() == "r-0ff1")
        .unwrap();
    assert!(row.tasks.is_empty(), "{:?}", row.tasks);
    let of_task = listed(&app, Some(id)).await.sessions;
    assert!(of_task.is_empty(), "no session is linked: {of_task:?}");

    report(
        &app,
        &session_event(
            id,
            "r-0ff1",
            "turn_finished",
            "idle",
            json!({ "turn": { "id": "r-0ff1:1", "outcome": "completed", "last": "Done." } }),
        ),
    )
    .await;
    let history = app.state.tasks().history_of(id).await.unwrap();
    assert!(
        !history
            .iter()
            .any(|line| line.text == "A turn ended with no plan"),
        "{history:?}"
    );
}

#[tokio::test]
pub(crate) async fn a_worker_that_ends_with_no_plan_is_noted_and_a_plan_clears_it() {
    let app = app(true).await;
    let id = task(&app, "the parser").await;
    let tasks = app.state.tasks();
    tasks
        .worker_started(id, "w-1", "Do the parser.")
        .await
        .unwrap();
    tasks.worker_ended("w-1").await.unwrap();
    let history = tasks.history_of(id).await.unwrap();
    assert_eq!(history.last().unwrap().text, "A turn ended with no plan");
    as_worker(
        &app,
        id,
        "plan_set",
        json!({ "steps": [{ "title": "Read it" }] }),
    )
    .await;
    let issue = tasks.issue(id).await.unwrap().unwrap();
    assert_eq!(issue.plan.unwrap().unplanned_turn, None, "a plan clears it");
    tasks.worker_started(id, "w-2", "Carry on.").await.unwrap();
    tasks.worker_ended("w-2").await.unwrap();
    assert_eq!(
        tasks.history_of(id).await.unwrap().last().unwrap().text,
        "Plan set: 1 step",
        "a task with a plan isn't noted"
    );
}

#[tokio::test]
pub(crate) async fn the_shared_mcp_list_names_each_session_to_this_server() {
    for on in [true, false] {
        let app = app(on).await;
        std::fs::write(
            app.dir().join("runner-mcp.json"),
            json!({ "mcpServers": {
                "emberhut": { "type": "http", "url": "https://hut.example.com/mcp/emberhut",
                              "headers": { "Authorization": "Bearer x" } },
                "docs": { "type": "http", "url": "https://docs.example/mcp" },
            } })
            .to_string(),
        )
        .unwrap();
        let answer = app
            .call(Call::get("/api/runner/mcp").header("authorization", &format!("Bearer {BUILD}")))
            .await;
        let servers = &answer.json()["mcpServers"];
        let header = &servers["emberhut"]["headers"][SESSION_HEADER];
        if on {
            assert_eq!(header, SESSION_HEADER_VALUE);
        } else {
            assert!(header.is_null(), "{servers}");
        }
        assert_eq!(servers["emberhut"]["headers"]["Authorization"], "Bearer x");
        assert!(servers["docs"].get("headers").is_none(), "{servers}");
    }
}

#[tokio::test]
pub(crate) async fn every_brief_asks_for_the_plan_only_when_plans_are_kept() {
    // The assistant's own rules.
    for on in [true, false] {
        let cli = crate::fake_cli::FakeCli::new(|_, line| {
            if crate::fake_cli::is_user(line) {
                vec![crate::fake_cli::init("s-1"), crate::fake_cli::result("Hi.")]
            } else {
                Vec::new()
            }
        });
        let config = emberhut_server::services::claude::ClaudeConfig {
            task_plans: on,
            ..crate::fake_cli::config(&[])
        };
        let app = TestApp::with_agent(crate::fake_cli::agent(&cli, config)).await;
        app.state
            .talk()
            .post(RoomId::MAIN, "Hello".to_owned())
            .await
            .unwrap();
        app.settle().await;
        let lines = cli.lines();
        let append = lines[0].1["request"]["appendSystemPrompt"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            append.contains(emberhut_server::services::tasks::ASSISTANT_RULE),
            on,
            "{append}"
        );
    }

    // A session the assistant starts for a task.
    for on in [true, false] {
        let app = app(on).await;
        app.runners.answer(
            "build",
            "/sessions",
            json!({ "id": "r-77", "turn": "r-77:1" }),
        );
        app.call_tool(
            "thread_start",
            json!({ "task_id": 12, "title": "Fix the build", "message": "The build fails on main.",
                    "cwd": "/work/app" }),
        )
        .await;
        let opened = app.runners.requests_to("/sessions")[0]
            .body
            .clone()
            .unwrap();
        let prompt = opened["prompt"].as_str().unwrap();
        if on {
            assert_eq!(
                prompt,
                format!(
                    "The build fails on main.\n\n{}",
                    emberhut_server::services::tasks::session_rule("T-12")
                )
            );
            assert!(prompt.contains("STATUS line as before"), "{prompt}");
        } else {
            assert_eq!(prompt, "The build fails on main.");
        }
    }

    // A worker's brief.
    for on in [true, false] {
        let cli = crate::workers::answering_cli("Done.");
        let spawner = std::sync::Arc::clone(&cli);
        let app = TestApp::with_dependencies(AuthMode::Enforce, move |deps, _| Dependencies {
            task_plans: on,
            worker_setup: emberhut_server::app::WorkerSetup {
                runtime: Some(emberhut_server::services::workers::WorkerRuntime {
                    spawner,
                    config: crate::workers::worker_config(),
                }),
                ..emberhut_server::app::WorkerSetup::default()
            },
            ..deps
        })
        .await;
        let id = crate::workers::assistant_task(&app, "the parser review", false).await;
        app.state.tasks().start_worker(id).await.unwrap();
        crate::workers::until_ended(&app, id).await;
        let brief = &cli.said()[0];
        assert_eq!(
            brief.contains(emberhut_server::services::tasks::WORKER_RULE),
            on,
            "{brief}"
        );
    }
}

#[tokio::test]
pub(crate) async fn a_steps_limits_count_characters_not_bytes() {
    let app = app(true).await;
    let id = task(&app, "the Arabic brief").await;
    let fits = "ب".repeat(150);
    let set = as_room(
        &app,
        None,
        "plan_set",
        json!({ "task": id.get(), "steps": [{ "title": fits, "note": "ت".repeat(1500) }] }),
    )
    .await;
    assert_ne!(set.is_error, Some(true), "{set:?}");
    let over = as_room(
        &app,
        None,
        "plan_step",
        json!({ "task": id.get(), "add": { "title": "ب".repeat(201) } }),
    )
    .await;
    assert!(error(&over).contains("200"), "{over:?}");
}

#[tokio::test]
pub(crate) async fn a_runner_event_is_taken_when_its_tasks_bookkeeping_fails() {
    let app = app(true).await;
    let id = task(&app, "the lexer").await;
    // Every link written from now on fails.
    let conn = sea_orm::Database::connect(app.url.as_str()).await.unwrap();
    sea_orm::ConnectionTrait::execute_unprepared(
        &conn,
        "CREATE TRIGGER no_links BEFORE INSERT ON links BEGIN SELECT RAISE(ABORT, 'no links'); END",
    )
    .await
    .unwrap();
    conn.close().await.unwrap();

    report(
        &app,
        &session_event(
            id,
            "r-b0b",
            "note",
            "running",
            json!({ "text": "Half way there" }),
        ),
    )
    .await;
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let log: Vec<String> = db
        .log()
        .recent(5)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        log.iter()
            .any(|line| line == "Runner session \"The lexer\": Half way there"),
        "the note is written all the same: {log:?}"
    );
    assert_eq!(page_runs(&app, id).await, 0);
}

/// How many times the links table was written since [`count_link_writes`].
async fn link_writes(app: &TestApp) -> i64 {
    let conn = sea_orm::Database::connect(app.url.as_str()).await.unwrap();
    let row = sea_orm::ConnectionTrait::query_one_raw(
        &conn,
        sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM link_writes",
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let n = row.try_get::<i64>("", "n").unwrap();
    conn.close().await.unwrap();
    n
}

/// Counts every write of the links table from now on.
async fn count_link_writes(app: &TestApp) {
    let conn = sea_orm::Database::connect(app.url.as_str()).await.unwrap();
    for statement in [
        "CREATE TABLE link_writes (at INTEGER)",
        "CREATE TRIGGER count_links BEFORE INSERT ON links BEGIN INSERT INTO link_writes VALUES (1); END",
    ] {
        sea_orm::ConnectionTrait::execute_unprepared(&conn, statement)
            .await
            .unwrap();
    }
    conn.close().await.unwrap();
}

#[tokio::test]
pub(crate) async fn a_session_is_linked_when_its_turn_is_sent_or_starts_not_on_every_event() {
    let app = app(true).await;
    let id = task(&app, "the lexer").await;
    count_link_writes(&app).await;
    let turn = json!({ "turn": { "id": "r-11c:1", "by": "assistant", "prompt": "Write it" } });
    report(
        &app,
        &session_event(id, "r-11c", "turn_queued", "queued", turn.clone()),
    )
    .await;
    report(
        &app,
        &session_event(id, "r-11c", "turn_started", "running", turn.clone()),
    )
    .await;
    assert_eq!(link_writes(&app).await, 2, "queued, then started");
    for doing in ["Reading", "Writing", "Testing"] {
        report(
            &app,
            &session_event(
                id,
                "r-11c",
                "activity",
                "running",
                json!({ "doing": doing }),
            ),
        )
        .await;
    }
    report(
        &app,
        &session_event(
            id,
            "r-11c",
            "note",
            "running",
            json!({ "text": "Half way" }),
        ),
    )
    .await;
    assert_eq!(
        link_writes(&app).await,
        2,
        "no write for activity or a note"
    );
    assert!(
        page(&app, id)
            .await
            .links
            .iter()
            .any(|link| link.reference == "r-11c")
    );

    // A session first seen mid-turn is linked once, by its first event.
    for _ in 0..3 {
        report(
            &app,
            &session_event(
                id,
                "r-22d",
                "activity",
                "running",
                json!({ "doing": "Reading" }),
            ),
        )
        .await;
    }
    assert_eq!(link_writes(&app).await, 3);
    assert!(
        page(&app, id)
            .await
            .links
            .iter()
            .any(|link| link.reference == "r-22d")
    );
}
