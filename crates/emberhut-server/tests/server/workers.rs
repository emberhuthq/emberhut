//! The assistant's workers: a task's worker started on the scripted CLI with the worker
//! endpoint and its token, its stream kept and read back as `w-<task>`, its tools on the
//! worker endpoint (`POST /mcp/worker`), and the state's `working`.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::mcp::{CallToolResult, RpcResponse};
use emberhut_core::task::{Owner, Status, TaskChange, TaskId, Who};
use emberhut_db::NewTask;
use emberhut_server::app::{Dependencies, WorkerSetup};
use emberhut_server::config::AuthMode;
use emberhut_server::services::claude::worker::WorkerConfig;
use emberhut_server::services::workers::WorkerRuntime;
use serde_json::{Value, json};

use crate::common::{Answer, Call, TestApp, text};
use crate::fake_cli::{FakeCli, init, is_user, result, step};

/// The worker endpoint's path.
const WORKER: &str = emberhut_api::paths::MCP_WORKER;

/// How the tests run workers.
pub(crate) fn worker_config() -> WorkerConfig {
    WorkerConfig {
        command: "agent-cli".into(),
        args: Vec::new(),
        model: Some("a-model".to_owned()),
        permission_mode: None,
        accounts: Vec::new(),
        mcp_url: format!("http://127.0.0.1:18990{WORKER}"),
        gate_url: format!("http://127.0.0.1:18990{}", emberhut_api::paths::ULTRA_GATE),
        workdir: Some("/work".into()),
    }
}

/// An app whose workers run on `cli`.
pub(crate) async fn worker_app(cli: &Arc<FakeCli>) -> TestApp {
    let spawner = Arc::clone(cli);
    TestApp::with_dependencies(AuthMode::Enforce, move |deps, _| Dependencies {
        worker_setup: WorkerSetup {
            runtime: Some(WorkerRuntime {
                spawner,
                config: worker_config(),
            }),
            ..WorkerSetup::default()
        },
        ..deps
    })
    .await
}

/// A CLI that answers a worker's brief with its session, a step and its answer.
pub(crate) fn answering_cli(answer: &'static str) -> Arc<FakeCli> {
    FakeCli::new(move |_, line| {
        if is_user(line) {
            vec![
                init("s-77"),
                step("Reading the code.", "Bash"),
                result(answer),
            ]
        } else {
            Vec::new()
        }
    })
}

/// An assistant's task with a brief; `ultra` turns ultra mode on.
pub(crate) async fn assistant_task(app: &TestApp, title: &str, ultra: bool) -> TaskId {
    let new = NewTask {
        title: title.to_owned(),
        body: Some(format!("Do {title} end to end.")),
        owner: Some(Owner::Assistant),
        ..NewTask::default()
    };
    let task = app.state.tasks().create(new, Who::Assistant).await.unwrap();
    if ultra {
        let change = TaskChange {
            ultra: Some(true),
            ..TaskChange::default()
        };
        app.state
            .tasks()
            .update(task.id, change, Who::Assistant, None)
            .await
            .unwrap();
    }
    task.id
}

/// Waits (polling) until the task's worker isn't running.
pub(crate) async fn until_ended(app: &TestApp, task: TaskId) {
    for _ in 0..500 {
        if !app.state.workers().is_running(task) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the worker didn't end");
}

/// A JSON-RPC request to the worker endpoint as `task`'s worker.
pub(crate) async fn worker_rpc(app: &TestApp, task: TaskId, method: &str, params: Value) -> Answer {
    let token = app.state.worker_tokens().for_task(task).unwrap();
    let body = json!({ "jsonrpc": "2.0", "id": 9, "method": method, "params": params });
    app.call(
        Call::post(WORKER)
            .header("authorization", &format!("Bearer {}", token.as_str()))
            .json(&body),
    )
    .await
}

/// `tools/call` of `name` as `task`'s worker: its result.
pub(crate) async fn worker_tool(
    app: &TestApp,
    task: TaskId,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let answer = worker_rpc(
        app,
        task,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: RpcResponse = answer.read_as();
    serde_json::from_value(reply.result.unwrap()).unwrap()
}

#[tokio::test]
async fn a_worker_runs_its_task_on_the_worker_endpoint_and_its_stream_is_its_transcript() {
    let cli = answering_cli("Read it all; nothing left.");
    let app = worker_app(&cli).await;
    let task = assistant_task(&app, "the parser review", false).await;
    app.state.tasks().start_worker(task).await.unwrap();
    until_ended(&app, task).await;

    let launch = &cli.launches()[0];
    let mcp = launch.flag("--mcp-config").unwrap();
    assert!(mcp.contains(WORKER), "{mcp}");
    assert!(
        mcp.contains("${EMBERHUT_MCP_TOKEN}"),
        "the token stays off the command line"
    );
    let token = app.state.worker_tokens().for_task(task).unwrap();
    assert_eq!(launch.var("EMBERHUT_MCP_TOKEN"), Some(token.as_str()));
    assert_eq!(launch.flag("--permission-mode"), Some("dontAsk"));
    assert!(
        launch.args.iter().any(|a| a == "Bash(git push:*)"),
        "outward commands are refused: {:?}",
        launch.args
    );
    let brief = &cli.said()[0];
    assert!(
        brief.contains(&format!("Task {}: the parser review", task.key())),
        "{brief}"
    );
    assert!(
        brief.contains("Do the parser review end to end."),
        "{brief}"
    );

    // It ended without task_done: the owner is asked whether to try again, Main hears it.
    let issue = app.state.tasks().issue(task).await.unwrap().unwrap();
    assert_eq!(issue.status, emberhut_api::tasks::TaskStatus::Waiting);
    let history = app.state.tasks().history_of(task).await.unwrap();
    assert!(history.iter().any(|h| h.text == "A worker started on it"));
    assert!(
        history
            .iter()
            .any(|h| h.text == "Stopped without finishing: Read it all; nothing left."),
        "{history:?}"
    );

    // Its steps, read back from its own stream, as the session w-<task>.
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::get(&format!("/api/runner/sessions/w-{}/transcript", task.get())).token(&session),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let body = answer.json();
    assert_eq!(body["session"]["id"], format!("w-{}", task.get()));
    assert_eq!(body["session"]["running"], false);
    let kinds: Vec<&str> = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["text", "tool", "end"]);
    assert_eq!(body["events"][1]["doing"], "Running: ls");
}

#[tokio::test]
pub(crate) async fn the_worker_endpoint_serves_a_worker_s_own_task_tools() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "the report", false).await;
    let other = assistant_task(&app, "something else", false).await;

    let refused = app
        .call(
            Call::post(WORKER)
                .header("authorization", "Bearer nope")
                .json(&json!({})),
        )
        .await;
    assert_eq!(refused.status, 401);

    let list = worker_rpc(&app, task, "tools/list", json!({})).await;
    let names: Vec<String> = list.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    for name in [
        "task",
        "task_note",
        "task_ask",
        "task_findings",
        "task_runner",
        "task_done",
        "recall",
    ] {
        assert!(names.iter().any(|n| n == name), "{name}: {names:?}");
    }
    assert!(
        !names.iter().any(|n| n == "ultra_plan"),
        "not an ultra task"
    );
    assert!(
        !names.iter().any(|n| n == "task_create"),
        "nothing that acts beyond its task"
    );

    let noted = worker_tool(
        &app,
        task,
        "task_note",
        json!({ "text": "Read the numbers." }),
    )
    .await;
    assert_eq!(text(&noted), format!("Noted on {}.", task.key()));
    let elsewhere = worker_tool(
        &app,
        task,
        "task_note",
        json!({ "text": "x", "task_id": other.get() }),
    )
    .await;
    assert!(
        text(&elsewhere).starts_with("Not noted:"),
        "{}",
        text(&elsewhere)
    );

    let runner = worker_tool(
        &app,
        task,
        "task_runner",
        json!({ "unit": "render-1.service" }),
    )
    .await;
    assert_eq!(
        text(&runner),
        format!(
            "{} runs on render-1: it shows Running while the unit is active.",
            task.key()
        )
    );
    let detail = app.state.tasks().detail(task).await.unwrap().unwrap();
    assert!(
        detail
            .history
            .iter()
            .any(|h| h.text == "Runs in the background as render-1"),
        "{:?}",
        detail.history
    );
    let bad = worker_tool(&app, task, "task_runner", json!({ "unit": "a b" })).await;
    assert_eq!(bad.is_error, Some(true));

    let mine = worker_tool(&app, task, "task", json!({})).await;
    assert!(text(&mine).contains("the report"));

    let done = worker_tool(
        &app,
        task,
        "task_done",
        json!({ "summary": "Sent the report." }),
    )
    .await;
    assert_eq!(text(&done), "Done.");
    assert_eq!(
        app.state.tasks().status_of(task).await.unwrap(),
        Some(Status::Done)
    );
}

#[tokio::test]
async fn the_state_shows_the_work_in_flight_waiting_first() {
    let app = worker_app(&answering_cli("done")).await;
    let queued = assistant_task(&app, "a queued job", false).await;
    let asking = assistant_task(&app, "a job that asks", false).await;
    worker_tool(
        &app,
        asking,
        "task_ask",
        json!({ "action": "Pick a name", "question": "Which name?", "options": ["Alpha", "Beta"] }),
    )
    .await;
    let session = app.sign_in().await;
    let state = app.call(Call::get("/api/state").token(&session)).await;
    assert_eq!(state.status, 200, "{}", state.text);
    let working = state.json()["working"].clone();
    let rows = working.as_array().unwrap();
    assert_eq!(rows[0]["task"], asking.get(), "{working}");
    assert_eq!(rows[0]["state"], "waiting");
    assert_eq!(rows[0]["action"], "Pick a name");
    assert_eq!(rows[0]["options"], json!(["Alpha", "Beta"]));
    assert!(
        rows.iter()
            .any(|r| r["task"] == queued.get() && r["state"] == "queued"),
        "{working}"
    );
    assert_eq!(state.json()["agentsNow"], json!([]));
}

#[tokio::test]
pub(crate) async fn the_worker_endpoint_reads_its_messages_as_the_room_s_does() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "a job", false).await;
    let token = app.state.worker_tokens().for_task(task).unwrap();
    let bearer = format!("Bearer {}", token.as_str());
    let notified = app
        .call(
            Call::post(WORKER)
                .header("authorization", &bearer)
                .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
        )
        .await;
    assert_eq!(notified.status, 202);
    let garbled = app
        .call(
            Call::post(WORKER)
                .header("authorization", &bearer)
                .raw("{ not json"),
        )
        .await;
    assert_eq!(garbled.status, 400);
    let huge = json!({ "jsonrpc": "2.0", "id": 1, "method": "x".repeat(3 * 1024 * 1024) });
    let big = app
        .call(
            Call::post(WORKER)
                .header("authorization", &bearer)
                .json(&huge),
        )
        .await;
    assert_eq!(big.status, 413);
}

/// A CLI that opens its session and then works on, never answering.
fn busy_cli() -> Arc<FakeCli> {
    FakeCli::new(|_, line| {
        if is_user(line) {
            vec![init("s-busy")]
        } else {
            Vec::new()
        }
    })
}

#[tokio::test]
async fn two_starts_of_one_task_at_once_spawn_one_worker() {
    let cli = busy_cli();
    let app = worker_app(&cli).await;
    let task = assistant_task(&app, "the importer", false).await;
    let workers = app.state.workers();
    let (first, second) = tokio::join!(workers.start(task, None), workers.start(task, None));
    first.unwrap();
    second.unwrap();
    assert_eq!(cli.launches().len(), 1, "exactly one CLI started");
    assert!(workers.is_running(task));
    workers.close_all(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn closing_the_workers_stops_each_one_and_waits_for_it() {
    let cli = busy_cli();
    let app = worker_app(&cli).await;
    let one = assistant_task(&app, "the importer", false).await;
    let two = assistant_task(&app, "the exporter", false).await;
    let workers = app.state.workers();
    workers.start(one, None).await.unwrap();
    workers.start(two, None).await.unwrap();
    assert!(workers.is_running(one) && workers.is_running(two));

    workers.close_all(Duration::from_secs(5)).await;
    assert!(!workers.is_running(one) && !workers.is_running(two));
    assert_eq!(cli.stopped(), [0, 1], "each CLI was stopped");

    let three = assistant_task(&app, "the report", false).await;
    workers.start(three, None).await.unwrap();
    assert_eq!(cli.launches().len(), 2, "none starts once they are closed");
}
