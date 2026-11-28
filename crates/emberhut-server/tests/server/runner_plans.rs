//! The runner sessions' endpoint (`POST` [`emberhut_api::paths::MCP_RUNNER`], `--task-plans`):
//! the server's own entry in the runners' shared MCP list, with the runner sessions' token; the
//! short list of tools the token reaches; and plan writes: a linked session's own tasks only, a
//! call naming no linked session the task it names.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_api::error::ErrorBody;
use emberhut_api::mcp::{CallToolResult, RpcResponse};
use emberhut_api::paths;
use emberhut_api::wire::mcp::{SERVER_NAME, SESSION_HEADER, SESSION_HEADER_VALUE};
use emberhut_core::task::{Owner, TaskId, Who};
use emberhut_core::{OsRandom, RoomId};
use emberhut_db::NewTask;
use emberhut_server::app::Dependencies;
use emberhut_server::config::AuthMode;
use emberhut_server::mcp::runner_token::RunnerToken;
use serde_json::{Value, json};

use crate::common::{Answer, Call, TestApp, text};

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

/// The runners' shared MCP list, as the build runner gets it.
async fn shared_list(app: &TestApp) -> Value {
    let answer = app
        .call(Call::get("/api/runner/mcp").header("authorization", &format!("Bearer {BUILD}")))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.json()["mcpServers"].clone()
}

/// The runner sessions' token, as the shared list hands it out.
async fn listed_token(app: &TestApp) -> String {
    let servers = shared_list(app).await;
    servers[SERVER_NAME]["headers"]["Authorization"]
        .as_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap()
        .to_owned()
}

/// One JSON-RPC request to `path` with the bearer `token` and, when given, the session header.
async fn rpc(
    app: &TestApp,
    path: &str,
    token: &str,
    session: Option<&str>,
    method: &str,
    params: Value,
) -> Answer {
    let mut request = Call::post(path)
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({ "jsonrpc": "2.0", "id": 4, "method": method, "params": params }));
    if let Some(session) = session {
        request = request.header(SESSION_HEADER, session);
    }
    app.call(request).await
}

/// `tools/call` of `name` on the runner sessions' endpoint: the answer's result.
async fn as_runner(
    app: &TestApp,
    token: &str,
    session: Option<&str>,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let answer = rpc(
        app,
        paths::MCP_RUNNER,
        token,
        session,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    serde_json::from_value(answer.read_as::<RpcResponse>().result.unwrap()).unwrap()
}

fn error(result: &CallToolResult) -> &str {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    text(result)
}

fn ok(result: &CallToolResult) -> &str {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    text(result)
}

/// A turn of the runner session `session` starts on `task`: the session is linked to it.
async fn start_turn(app: &TestApp, task: TaskId, session: &str) {
    let answer = app
        .call(
            Call::post("/api/runner/events")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(&json!({
                    "machine": "build", "kind": "turn_started", "at": "2026-11-25T10:00:00.000Z",
                    "session": { "id": session, "title": "The work", "project": "",
                                 "status": "running", "meta": { "task": task.key() } },
                    "turn": { "id": format!("{session}:1"), "started": "2026-11-25T10:00:00.000Z" },
                })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
}

#[tokio::test]
pub(crate) async fn the_shared_list_has_the_server_s_own_entry_only_with_plans() {
    let off = app(false).await;
    assert_eq!(shared_list(&off).await, json!({}), "as before plans");
    std::fs::write(
        off.dir().join("runner-mcp.json"),
        json!({ "mcpServers": { "docs": { "type": "http", "url": "https://docs.example/mcp" } } })
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        shared_list(&off).await,
        json!({ "docs": { "type": "http", "url": "https://docs.example/mcp" } })
    );

    let on = app(true).await;
    let servers = shared_list(&on).await;
    let token = on.state.runner_token().get().unwrap();
    assert_eq!(
        servers,
        json!({ SERVER_NAME: {
            "type": "http",
            "url": format!("http://127.0.0.1:18990{}", paths::MCP_RUNNER),
            "headers": {
                "Authorization": format!("Bearer {}", token.as_str()),
                SESSION_HEADER: SESSION_HEADER_VALUE,
            },
        } })
    );
    assert_eq!(
        listed_token(&on).await,
        token.as_str(),
        "the same token each time"
    );

    // The operator's own servers are listed beside it.
    std::fs::write(
        on.dir().join("runner-mcp.json"),
        json!({ "mcpServers": { "docs": { "type": "http", "url": "https://docs.example/mcp" } } })
            .to_string(),
    )
    .unwrap();
    let servers = shared_list(&on).await;
    assert_eq!(servers.as_object().unwrap().len(), 2, "{servers}");
    assert_eq!(
        servers["docs"],
        json!({ "type": "http", "url": "https://docs.example/mcp" })
    );
    assert!(servers[SERVER_NAME]["url"].is_string(), "{servers}");

    // An entry the operator listed for the server's room endpoint, under another name, is kept
    // and none is added; one for the runner sessions' endpoint gets the token.
    std::fs::write(
        on.dir().join("runner-mcp.json"),
        json!({ "mcpServers": {
            "hut": { "type": "http", "url": "https://hut.example.com/mcp/emberhut",
                     "headers": { "Authorization": "Bearer x" } },
        } })
        .to_string(),
    )
    .unwrap();
    let servers = shared_list(&on).await;
    assert_eq!(
        servers,
        json!({ "hut": { "type": "http", "url": "https://hut.example.com/mcp/emberhut",
                         "headers": { "Authorization": "Bearer x",
                                      SESSION_HEADER: SESSION_HEADER_VALUE } } })
    );
    std::fs::write(
        on.dir().join("runner-mcp.json"),
        json!({ "mcpServers": {
            SERVER_NAME: { "type": "http", "url": "https://hut.example.com/mcp/runner" },
        } })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        shared_list(&on).await,
        json!({ SERVER_NAME: { "type": "http", "url": "https://hut.example.com/mcp/runner",
                               "headers": { SESSION_HEADER: SESSION_HEADER_VALUE,
                                            "Authorization": format!("Bearer {}", token.as_str()) } } })
    );
}

#[tokio::test]
pub(crate) async fn the_runner_sessions_token_reaches_only_its_task_and_plan_tools() {
    let app = app(true).await;
    let token = listed_token(&app).await;
    let list = rpc(
        &app,
        paths::MCP_RUNNER,
        &token,
        None,
        "tools/list",
        json!({}),
    )
    .await;
    assert_eq!(list.status, 200, "{}", list.text);
    let names: Vec<String> = list.read_as::<RpcResponse>().result.unwrap()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["task", "plan_set", "plan_step", "plan_read"]);
    let send = as_runner(
        &app,
        &token,
        None,
        "thread_send",
        json!({ "thread": "x", "text": "hi" }),
    )
    .await;
    assert_eq!(error(&send), "MCP error -32602: Tool thread_send not found");

    // Only this process's token: a stranger's, an old process's, a room's are unknown callers,
    // and the runner sessions' token is no room's.
    let old = RunnerToken::new(std::sync::Arc::new(OsRandom))
        .get()
        .unwrap();
    let room = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    for stranger in ["not a token", old.as_str(), room.as_str()] {
        let answer = rpc(&app, paths::MCP_RUNNER, stranger, None, "ping", json!({})).await;
        assert_eq!(answer.status, 401, "{stranger}");
        assert_eq!(answer.read_as::<ErrorBody>().error, "unknown caller");
    }
    let none = app
        .call(
            Call::post(paths::MCP_RUNNER)
                .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })),
        )
        .await;
    assert_eq!(none.status, 401);
    let as_room = rpc(&app, paths::MCP_ROOM, &token, None, "ping", json!({})).await;
    assert_eq!(as_room.status, 401);
    assert_eq!(as_room.read_as::<ErrorBody>().error, "unknown caller");

    // With the flag off no token is handed out, so none is known.
    let off = self::app(false).await;
    let answer = rpc(&off, paths::MCP_RUNNER, &token, None, "ping", json!({})).await;
    assert_eq!(answer.status, 401);
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one story: unnamed sessions, a linked one, then a room's token"
)]
pub(crate) async fn a_runner_session_writes_its_linked_task_s_plan_or_the_one_it_names() {
    let app = app(true).await;
    let token = listed_token(&app).await;
    let mine = task(&app, "the lexer").await;
    let other = task(&app, "the parser").await;

    // No session, an unexpanded one, one the server doesn't know: as a room token, a write
    // names its task; left out, the call is told the session header names none.
    for session in [None, Some(SESSION_HEADER_VALUE), Some("r-0ff")] {
        let write = as_runner(
            &app,
            &token,
            session,
            "plan_set",
            json!({ "task": other.get(), "steps": [{ "title": "Draft" }] }),
        )
        .await;
        assert!(
            ok(&write).starts_with(&format!("Plan of {}", other.key())),
            "{session:?}: {write:?}"
        );
        assert_eq!(
            app.state
                .tasks()
                .history_of(other)
                .await
                .unwrap()
                .last()
                .unwrap()
                .who,
            Who::Assistant
        );
        let step = as_runner(
            &app,
            &token,
            session,
            "plan_step",
            json!({ "add": { "title": "Tokens" } }),
        )
        .await;
        let refused = error(&step);
        assert!(
            refused.starts_with("Say which task (task: its number or key, T-12)")
                && refused.contains(SESSION_HEADER),
            "{session:?}: {refused}"
        );
        let read = as_runner(
            &app,
            &token,
            session,
            "plan_read",
            json!({ "task": mine.key() }),
        )
        .await;
        assert!(ok(&read).contains("No plan yet"), "{session:?}");
        let unnamed = as_runner(&app, &token, session, "plan_read", json!({})).await;
        assert!(error(&unnamed).contains(SESSION_HEADER), "{session:?}");
    }
    let full = as_runner(&app, &token, None, "task", json!({ "id": mine.get() })).await;
    let full: Value = serde_json::from_str(ok(&full)).unwrap();
    assert_eq!(full["title"], "the lexer", "{full}");

    // Linked to its task, it writes that one's plan, the task left out.
    start_turn(&app, mine, "r-1e8").await;
    let set = as_runner(
        &app,
        &token,
        Some("r-1e8"),
        "plan_set",
        json!({ "steps": [{ "title": "Tokens", "state": "doing" }] }),
    )
    .await;
    assert!(
        ok(&set).starts_with(&format!("Plan of {} \"the lexer\"", mine.key())),
        "{set:?}"
    );
    let ticked = as_runner(
        &app,
        &token,
        Some("r-1e8"),
        "plan_step",
        json!({ "step": "Tokens", "state": "done" }),
    )
    .await;
    assert!(ok(&ticked).contains("[x]"), "{ticked:?}");
    let history = app.state.tasks().history_of(mine).await.unwrap();
    let last = history.last().unwrap();
    assert_eq!(
        (last.text.as_str(), last.who),
        ("Ticked: Tokens", Who::Assistant)
    );
    let not_its = as_runner(
        &app,
        &token,
        Some("r-1e8"),
        "plan_set",
        json!({ "task": other.key(), "steps": [{ "title": "Nope" }] }),
    )
    .await;
    assert!(error(&not_its).starts_with(&format!("You work on {}", mine.key())));

    // The room's token is as before: with no session it writes any task's plan.
    let room = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    let answer = rpc(
        &app,
        paths::MCP_ROOM,
        room.as_str(),
        None,
        "tools/call",
        json!({ "name": "plan_set",
                "arguments": { "task": other.get(), "steps": [{ "title": "Read it" }] } }),
    )
    .await;
    let room_set: CallToolResult =
        serde_json::from_value(answer.read_as::<RpcResponse>().result.unwrap()).unwrap();
    assert!(ok(&room_set).starts_with(&format!("Plan of {}", other.key())));
}

#[tokio::test]
pub(crate) async fn the_runner_sessions_endpoint_reads_its_messages_as_the_room_s_does() {
    let app = app(true).await;
    let token = listed_token(&app).await;
    let bearer = format!("Bearer {token}");
    let notified = app
        .call(
            Call::post(paths::MCP_RUNNER)
                .header("authorization", &bearer)
                .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
        )
        .await;
    assert_eq!(notified.status, 202);
    let garbled = app
        .call(
            Call::post(paths::MCP_RUNNER)
                .header("authorization", &bearer)
                .raw("{ not json"),
        )
        .await;
    assert_eq!(garbled.status, 400);
    let huge = json!({ "jsonrpc": "2.0", "id": 1, "method": "x".repeat(3 * 1024 * 1024) });
    let big = app
        .call(
            Call::post(paths::MCP_RUNNER)
                .header("authorization", &bearer)
                .json(&huge),
        )
        .await;
    assert_eq!(big.status, 413);
    let hello = rpc(
        &app,
        paths::MCP_RUNNER,
        &token,
        None,
        "initialize",
        json!({}),
    )
    .await;
    assert_eq!(hello.status, 200, "{}", hello.text);
    assert_eq!(
        hello.read_as::<RpcResponse>().result.unwrap()["serverInfo"]["name"],
        SERVER_NAME
    );
}
