//! The assistant's runner tools: `thread_start`, `session_read`, `session_send`,
//! `session_send_file`, `session_stop`, `machines` and the account tool
//! ([`emberhut_api::wire::mcp::ACCOUNT_TOOL`]); and the runner side of `threads` and `thread_send`.

use emberhut_api::wire::mcp::ACCOUNT_TOOL;
use emberhut_server::services::runner_api::by;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};

const BUILD: &str = "token-of-build";

async fn app() -> TestApp {
    let app = TestApp::new().await;
    app.runners.on(&[("build", "Build box", BUILD)]);
    app
}

async fn report(app: &TestApp, path: &str, body: &Value) {
    let answer = app
        .call(
            Call::post(path)
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(body),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
}

async fn session(app: &TestApp, id: &str, direct: bool) {
    report(
        app,
        "/api/runner/events",
        &json!({
            "machine": "build", "kind": "activity", "at": "2026-11-25T10:00:00.000Z",
            "session": { "id": id, "title": "Fix the build", "project": "", "status": "running",
                         "account": "work", "cwd": "/work/app", "meta": { "direct": direct } },
        }),
    )
    .await;
}

#[tokio::test]
async fn thread_start_starts_a_followed_session_on_a_runner_with_room() {
    let app = app().await;
    app.runners.answer(
        "build",
        "/sessions",
        json!({ "id": "r-77", "turn": "r-77:1", "account": "work", "cwd": "/work/app", "branch": "fix", "model": "model-a" }),
    );
    let started = app
        .call_tool(
            "thread_start",
            json!({ "task_id": 12, "title": "Fix the build", "message": "The build fails on main.",
                    "cwd": "/work/app", "worktree": "fix" }),
        )
        .await;
    assert_eq!(
        text(&started),
        "Session r-77 started on the runner on the Build box (work, model-a) in /work/app on \
         branch fix, followed. The user can read it and reply in the app."
    );
    let opened = app.runners.requests_to("/sessions")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(opened["prompt"], "The build fails on main.");
    assert_eq!(opened["meta"], json!({ "task": 12 }));
    assert_eq!(opened["effort"], "high");
    assert_eq!(opened["worktree"], json!({ "branch": "fix" }));
    let followed = app.state.threads().followed().await.unwrap();
    assert_eq!(followed[0].id, "r-77");
    assert_eq!(followed[0].why, "Fix the build");

    let gb = 1024.0 * 1024.0 * 1024.0;
    report(
        &app,
        "/api/runner/machine",
        &json!({ "machine": "build", "disks": [{ "path": "/", "total": 100.0 * gb, "free": 1.0 * gb }] }),
    )
    .await;
    let skipped = app
        .call_tool(
            "thread_start",
            json!({ "task_id": 12, "title": "Again", "message": "Go", "cwd": "/work/app" }),
        )
        .await;
    assert!(
        text(&skipped).starts_with(
            "Not started: the Build box (build) was skipped: / has only 1.0 GB free (99% full)."
        ),
        "{}",
        text(&skipped)
    );
    let forced = app
        .call_tool(
            "thread_start",
            json!({ "task_id": 12, "title": "Again", "message": "Go", "cwd": "/work/app", "force": true }),
        )
        .await;
    assert!(text(&forced).starts_with("Session r-77 started"));
    let no_folder = app
        .call_tool(
            "thread_start",
            json!({ "task_id": 1, "title": "T", "message": "M", "force": true }),
        )
        .await;
    assert_eq!(
        text(&no_folder),
        "Not started: pass cwd, the folder on build the work is in."
    );
    let elsewhere = app
        .call_tool(
            "thread_start",
            json!({ "task_id": 1, "title": "T", "message": "M", "machine": "nope", "cwd": "/" }),
        )
        .await;
    assert_eq!(
        text(&elsewhere),
        "Not started: no runner called nope (there are build)."
    );
}

#[tokio::test]
async fn session_read_reads_a_sessions_last_steps_plainly() {
    let app = app().await;
    session(&app, "r-1a", true).await;
    app.runners.answer(
        "build",
        "/sessions/r-1a/transcript",
        json!({
            "session": { "title": "Fix the build", "status": "running", "running": true,
                         "meta": { "direct": true }, "account": "work", "cwd": "/work/app" },
            "events": [
                { "kind": "user", "by": by::PERSON, "text": "Fix it" },
                { "kind": "tool", "doing": "Running tests", "summary": "cargo test" },
                { "kind": "text", "text": "Done." },
            ],
        }),
    );
    let read = app
        .call_tool("session_read", json!({ "thread": "r-1a", "limit": 10 }))
        .await;
    assert_eq!(
        text(&read),
        "Fix the build [running, running] (the person's own session) on the Build box's runner, \
         account work, in /work/app\n[owner] Fix it\n  · Running tests: cargo test\n[agent] Done."
    );
    assert!(
        app.runners.requests_to("/sessions/r-1a/transcript")[0]
            .path
            .ends_with("?limit=10")
    );
    app.runners
        .refuse("build", "/sessions/r-1a/transcript", "gone");
    let failed = app
        .call_tool("session_read", json!({ "thread": "r-1a" }))
        .await;
    assert_eq!(text(&failed), "Couldn't read it: gone");
}

#[tokio::test]
async fn session_send_writes_into_a_session_as_the_assistant() {
    let app = app().await;
    session(&app, "r-1a", true).await;
    app.runners.answer(
        "build",
        "/sessions/r-1a/send",
        json!({ "turn": "r-1a:3", "queued": true }),
    );
    let sent = app
        .call_tool(
            "session_send",
            json!({ "thread": "r-1a", "message": "The answer is B." }),
        )
        .await;
    assert_eq!(
        text(&sent),
        "Sent to the user's own session r-1a (queued behind the running turn)."
    );
    let body = app.runners.requests_to("/sessions/r-1a/send")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(
        body,
        json!({ "prompt": "The answer is B.", "by": by::ASSISTANT })
    );
    app.runners.refuse("build", "/sessions/r-1a/send", "closed");
    let refused = app
        .call_tool("session_send", json!({ "thread": "r-1a", "message": "Hi" }))
        .await;
    assert_eq!(
        text(&refused),
        "Not sent: closed. Check the session's id with the threads tool."
    );
}

#[tokio::test]
async fn session_send_file_puts_a_file_in_a_sessions_inbox() {
    let app = app().await;
    session(&app, "r-1a", false).await;
    let home = app.dir().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("design.png"), [1_u8; 2048]).unwrap();
    app.runners.answer(
        "build",
        "/sessions/r-1a/files",
        json!({ "path": "/work/app/.inbox/design.png", "size": 2048 }),
    );
    let sent = app
        .call_tool(
            "session_send_file",
            json!({ "thread": "r-1a", "path": "~/design.png" }),
        )
        .await;
    assert_eq!(
        text(&sent),
        "Landed at /work/app/.inbox/design.png (2 KB) on the Build box, in session r-1a's \
         inbox; it's told on its next turn."
    );
    assert_eq!(app.runners.uploads()[0].2.len(), 2048);
    let unknown = app
        .call_tool(
            "session_send_file",
            json!({ "thread": "r-9", "path": "~/design.png" }),
        )
        .await;
    assert_eq!(text(&unknown), "Not sent: no session r-9 on your runners");
}

#[tokio::test]
async fn session_stop_stops_a_turn_or_the_whole_session() {
    let app = app().await;
    session(&app, "r-1a", false).await;
    app.runners.answer(
        "build",
        "/sessions/r-1a/interrupt",
        json!({ "running": true }),
    );
    let stopped = app
        .call_tool("session_stop", json!({ "thread": "r-1a" }))
        .await;
    assert_eq!(text(&stopped), "Stopping it now.");
    let closed = app
        .call_tool("session_stop", json!({ "thread": "r-1a", "stop": true }))
        .await;
    assert_eq!(text(&closed), "Nothing was running.");
    assert_eq!(app.runners.requests_to("/sessions/r-1a/stop").len(), 1);
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let lines: Vec<String> = db
        .log()
        .recent(2)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert_eq!(
        lines,
        [
            "Stopped runner session r-1a",
            "Interrupted runner session r-1a"
        ]
    );
}

#[tokio::test]
async fn machines_tells_what_each_machine_has_left() {
    let app = app().await;
    let gb = 1024.0 * 1024.0 * 1024.0;
    report(
        &app,
        "/api/runner/machine",
        &json!({ "machine": "build", "cpu": { "cores": 8, "load1": 0.5, "usage": 0.1 },
                 "memory": { "total": 64.0 * gb, "available": 48.0 * gb } }),
    )
    .await;
    let all = app.call_tool("machines", json!({})).await;
    let answer: Value = serde_json::from_str(text(&all)).unwrap();
    assert_eq!(
        answer["summary"][0],
        "Build box (build): ok · load 0.5 on 8 cores (10% busy) · 48.0 GB of 64.0 GB memory available"
    );
    assert_eq!(answer["machines"][0]["state"], "ok");
    assert!(answer["machines"][0].get("history").is_none());
    let with_history = app
        .call_tool("machines", json!({ "machine": "build", "history": true }))
        .await;
    let answer: Value = serde_json::from_str(text(&with_history)).unwrap();
    assert_eq!(
        answer["machines"][0]["history"].as_array().unwrap().len(),
        1
    );
    let unknown = app
        .call_tool("machines", json!({ "machine": "nope" }))
        .await;
    assert_eq!(text(&unknown), "No runner machine nope (there are build).");
}

#[tokio::test]
async fn a_session_id_that_is_more_than_hex_digits_never_reaches_a_runner() {
    let app = app().await;
    session(&app, "r-1a", false).await;
    let before = app.runners.requests().len();
    for thread in [
        "../x?",
        "r-1a/../../jobs/stop?",
        "r-../off#",
        "r-1a%2f..",
        "R-1A",
    ] {
        let read = app
            .call_tool("session_read", json!({ "thread": thread }))
            .await;
        assert!(text(&read).starts_with("Couldn't read it: not a runner session's id"));
        let sent = app
            .call_tool("session_send", json!({ "thread": thread, "message": "Hi" }))
            .await;
        assert!(text(&sent).starts_with("Not sent: not a runner session's id"));
        let stopped = app
            .call_tool("session_stop", json!({ "thread": thread, "stop": true }))
            .await;
        assert!(text(&stopped).starts_with("Couldn't stop it: not a runner session's id"));
        let file = app
            .call_tool(
                "session_send_file",
                json!({ "thread": thread, "path": "~/design.png" }),
            )
            .await;
        assert!(text(&file).ends_with("is not a runner session's id"));
        let via_threads = app
            .call_tool(
                "thread_send",
                json!({ "machine": "build", "thread": thread, "message": "Hi" }),
            )
            .await;
        assert!(text(&via_threads).starts_with("Not sent: not a runner session's id"));
    }
    assert_eq!(
        app.runners.requests().len(),
        before,
        "no request reached the runner: {:?}",
        app.runners.requests()
    );
}

#[tokio::test]
async fn the_account_tool_shows_and_pins_the_assistants_account() {
    let app = app().await;
    app.runners.answer(
        "build",
        "/accounts",
        json!({ "accounts": [{ "id": "work", "name": "Work", "loggedIn": true, "used5h": 20.0 }] }),
    );
    app.state.runners().tick().await;
    let look = app.call_tool(ACCOUNT_TOOL, json!({})).await;
    let answer: Value = serde_json::from_str(text(&look)).unwrap();
    assert_eq!(answer["mode"], "auto");
    assert_eq!(answer["accounts"][0]["id"], "work");
    let pinned = app
        .call_tool(ACCOUNT_TOOL, json!({ "account": "work" }))
        .await;
    let answer: Value = serde_json::from_str(text(&pinned)).unwrap();
    assert_eq!(answer["pin"], "work");
    assert_eq!(answer["waits"], json!([]));
    let refused = app
        .call_tool(ACCOUNT_TOOL, json!({ "account": "other" }))
        .await;
    assert_eq!(
        text(&refused),
        "No account \"other\". Accounts: work, or auto."
    );
    let auto = app
        .call_tool(ACCOUNT_TOOL, json!({ "account": "auto" }))
        .await;
    let answer: Value = serde_json::from_str(text(&auto)).unwrap();
    assert_eq!(answer["mode"], "auto");
}

#[tokio::test]
async fn thread_send_to_a_runner_session_goes_to_its_runner() {
    let app = app().await;
    session(&app, "r-1a2b3c4d", false).await;
    app.runners.answer(
        "build",
        "/sessions/r-1a2b3c4d/send",
        json!({ "turn": "r-1a2b3c4d:2", "queued": true }),
    );
    // A runner session's id goes to its runner whatever machine was written.
    let sent = app
        .call_tool(
            "thread_send",
            json!({ "machine": "laptop", "thread": "r-1a2b3c4d", "message": "Carry on" }),
        )
        .await;
    assert_eq!(
        text(&sent),
        "Sent to runner session r-1a2b3c4d as its next turn (queued behind the running one)."
    );
    let body = app.runners.requests_to("/sessions/r-1a2b3c4d/send")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(body, json!({ "prompt": "Carry on", "by": by::ASSISTANT }));
    assert_eq!(
        app.coding_app.sent().len(),
        0,
        "nothing went to the coding-thread app"
    );

    // A runner named as the machine takes any id.
    app.runners
        .answer("build", "/sessions/r-77/send", json!({ "turn": "r-77:1" }));
    let named = app
        .call_tool(
            "thread_send",
            json!({ "machine": "build", "thread": "r-77", "message": "Hello" }),
        )
        .await;
    assert_eq!(
        text(&named),
        "Sent to runner session r-77 as its next turn."
    );
    app.runners
        .refuse("build", "/sessions/r-77/send", "no such session");
    let refused = app
        .call_tool(
            "thread_send",
            json!({ "machine": "build", "thread": "r-77", "message": "Hello" }),
        )
        .await;
    assert_ne!(refused.is_error, Some(true));
    assert_eq!(
        text(&refused),
        "Not sent: no such session. Check the session's id with the threads tool."
    );
}

#[tokio::test]
async fn threads_lists_the_runners_accounts_and_machines() {
    let app = app().await;
    app.runners.answer(
        "build",
        "/accounts",
        json!({ "accounts": [{ "id": "work", "name": "Work", "loggedIn": true, "used5h": 20.0 }] }),
    );
    app.state.runners().tick().await;
    let gb = 1024.0 * 1024.0 * 1024.0;
    report(
        &app,
        "/api/runner/machine",
        &json!({ "machine": "build", "cpu": { "cores": 8, "load1": 0.5, "usage": 0.1 },
                 "memory": { "total": 64.0 * gb, "available": 48.0 * gb } }),
    )
    .await;
    let listed = app.call_tool("threads", json!({})).await;
    assert_ne!(listed.is_error, Some(true), "{listed:?}");
    let answer: Value = serde_json::from_str(text(&listed)).unwrap();
    let account = &answer["runnerAccounts"][0];
    assert_eq!(account["id"], "work");
    assert_eq!(account["machine"], "build");
    assert_eq!(account["place"], "Build box");
    assert_eq!(account["used5h"], 20.0);
    assert!(account.get("login").is_none());
    assert_eq!(
        answer["machines"],
        json!([
            "Build box (build): ok · load 0.5 on 8 cores (10% busy) · 48.0 GB of 64.0 GB memory available"
        ])
    );
}
